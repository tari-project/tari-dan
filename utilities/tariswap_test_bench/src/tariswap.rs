//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::ShardId;
use tari_dan_wallet_sdk::{apis::key_manager::TRANSACTION_BRANCH, models::Account};
use tari_template_lib::{
    args,
    models::{Amount, ComponentAddress},
    prelude::XTR2,
};
use tari_transaction::Transaction;

use crate::{faucet::Faucet, runner::Runner};

pub struct TariSwap {
    pub component_address: ComponentAddress,
}

impl Runner {
    pub async fn create_tariswaps(
        &mut self,
        in_account: &Account,
        faucet: &Faucet,
        num_tariswaps: usize,
    ) -> anyhow::Result<Vec<TariSwap>> {
        let key = self.sdk.key_manager_api().derive_key(TRANSACTION_BRANCH, 0)?;
        let mut builder = Transaction::builder().fee_transaction_pay_from_component(
            in_account.address.as_component_address().unwrap(),
            Amount(1000 * num_tariswaps as i64),
        );
        for _ in 0..num_tariswaps {
            builder = builder.call_function(self.tariswap_template.address, "new", args![
                XTR2,
                faucet.resource_address,
                Amount(1)
            ]);
        }
        let transaction = builder.sign(&key.key).build();

        let finalize = self.submit_transaction_and_wait(transaction).await?;
        let diff = finalize.result.accept().unwrap();
        Ok(diff
            .up_iter()
            .filter_map(|(addr, value)| {
                addr.as_component_address()
                    .filter(|_| value.substate_value().component().unwrap().module_name == "TariSwapPool")
            })
            .map(|component_address| TariSwap { component_address })
            .collect())
    }

    pub async fn add_liquidity(
        &mut self,
        tariswaps: &[TariSwap],
        accounts: &[Account],
        amount_a: Amount,
        amount_b: Amount,
        faucet: &Faucet,
    ) -> anyhow::Result<()> {
        let key = self.sdk.key_manager_api().derive_key(TRANSACTION_BRANCH, 0)?;

        let mut tx_ids = vec![];
        // Batch these otherwise we can break consensus (proposed with locked object)
        for i in 0..5 {
            for (i, tariswap) in tariswaps.iter().enumerate().skip(i * 200).take(200) {
                let account = &accounts[i % accounts.len()];
                let transaction = Transaction::builder()
                    .with_input_refs(vec![
                        // Use resources as input refs to allow concurrent access.
                        ShardId::from_address(&faucet.resource_address.into(), 0),
                        ShardId::from_address(&XTR2.into(), 0),
                    ])
                    .fee_transaction_pay_from_component(account.address.as_component_address().unwrap(), Amount(1000))
                    .call_method(account.address.as_component_address().unwrap(), "withdraw", args![
                        XTR2, amount_a
                    ])
                    .put_last_instruction_output_on_workspace("a")
                    .call_method(account.address.as_component_address().unwrap(), "withdraw", args![
                        faucet.resource_address,
                        amount_b
                    ])
                    .put_last_instruction_output_on_workspace("b")
                    .call_method(tariswap.component_address, "add_liquidity", args![
                        Workspace("a"),
                        Workspace("b")
                    ])
                    .put_last_instruction_output_on_workspace("lp")
                    .call_method(account.address.as_component_address().unwrap(), "deposit", args![
                        Workspace("lp")
                    ])
                    .sign(&key.key)
                    .build();

                tx_ids.push(self.submit_transaction(transaction).await?);
            }

            for tx_id in tx_ids.drain(..) {
                self.wait_for_transaction(tx_id).await?;
            }
        }

        Ok(())
    }

    pub async fn do_tariswap_swaps(
        &mut self,
        tariswaps: &[TariSwap],
        accounts: &[Account],
        amount_a_for_b: Amount,
        amount_b_for_a: Amount,
        faucet: &Faucet,
    ) -> anyhow::Result<()> {
        let key = self.sdk.key_manager_api().derive_key(TRANSACTION_BRANCH, 0)?;

        let mut tx_ids = vec![];
        // Swap XTR2 for faucet
        // Batch these otherwise we can break consensus (proposed with locked object)
        for i in 0..5 {
            for (i, account) in accounts.iter().enumerate().skip(i * 200).take(200) {
                let tariswap = &tariswaps[i % tariswaps.len()];
                let transaction = Transaction::builder()
                    // Use resources as input refs to allow concurrent access.
                    .with_input_refs(vec![
                        ShardId::from_address(&faucet.resource_address.into(), 0),
                        ShardId::from_address(&XTR2.into(), 0),
                    ])
                    .fee_transaction_pay_from_component(account.address.as_component_address().unwrap(), Amount(1000))
                    .call_method(tariswap.component_address, "get_pool_balance", args![ XTR2, ])
                    .call_method(tariswap.component_address, "get_pool_balance", args![ faucet.resource_address, ])
                    .call_method(tariswap.component_address, "get_pool_ratio", args![XTR2, Amount(1000)])
                    .call_method(tariswap.component_address, "get_pool_ratio", args![faucet.resource_address, Amount(1000)])
                    .call_method(account.address.as_component_address().unwrap(), "withdraw", args![
                    XTR2, amount_a_for_b
                ])
                    .put_last_instruction_output_on_workspace("a")
                    .call_method(tariswap.component_address, "swap", args![
                    Workspace("a"),
                    faucet.resource_address,
                ])
                    .put_last_instruction_output_on_workspace("swapped")
                    .call_method(account.address.as_component_address().unwrap(), "deposit", args![
                    Workspace("swapped")
                ])
                    .sign(&key.key)
                    .build();

                tx_ids.push(self.submit_transaction(transaction).await?);
            }

            for (j, tx_id) in tx_ids.drain(..).enumerate() {
                let result = self.wait_for_transaction(tx_id).await?;
                let balance_a = result.execution_results[0].decode::<Amount>()?;
                let balance_b = result.execution_results[1].decode::<Amount>()?;
                let ratio_a = result.execution_results[2].decode::<Amount>()?;
                let ratio_b = result.execution_results[3].decode::<Amount>()?;
                let amount_swapped = amount_a_for_b.value() as f64 * (ratio_b.value() as f64 / 1000.0);
                log::info!(
                    "Swap {n} for {amount_a_for_b} XTR2 -> {amount_swapped} FAUCET @ {ratio_a}:{ratio_b} | pool \
                     liquidity: {balance_a} XTR2 {balance_b} FAUCET",
                    n = (i + 1) * (j + 1)
                );
            }
        }

        // Swap faucet for XTR2
        for i in 0..5 {
            for (i, account) in accounts.iter().enumerate().skip(i * 200).take(200) {
                let tariswap = &tariswaps[i % tariswaps.len()];
                let transaction = Transaction::builder()
                    // Use resources as input refs to allow concurrent access.
                    .with_input_refs(vec![
                        ShardId::from_address(&faucet.resource_address.into(), 0),
                        ShardId::from_address(&XTR2.into(), 0),
                    ])
                    .fee_transaction_pay_from_component(account.address.as_component_address().unwrap(), Amount(1000))
                    .call_method(tariswap.component_address, "get_pool_balance", args![XTR2])
                    .call_method(tariswap.component_address, "get_pool_balance", args![faucet.resource_address])
                    .call_method(tariswap.component_address, "get_pool_ratio", args![XTR2, Amount(1000)])
                    .call_method(tariswap.component_address, "get_pool_ratio", args![faucet.resource_address, Amount(1000)])
                    .call_method(account.address.as_component_address().unwrap(), "withdraw", args![
                        faucet.resource_address, amount_b_for_a
                    ])
                    .put_last_instruction_output_on_workspace("b")
                    .call_method(tariswap.component_address, "swap", args![
                        Workspace("b"),
                        XTR2,
                    ])
                    .put_last_instruction_output_on_workspace("swapped")
                    .call_method(account.address.as_component_address().unwrap(), "deposit", args![
                        Workspace("swapped")
                    ])
                    .sign(&key.key)
                    .build();

                tx_ids.push(self.submit_transaction(transaction).await?);
            }

            for (j, tx_id) in tx_ids.drain(..).enumerate() {
                let result = self.wait_for_transaction(tx_id).await?;
                let balance_a = result.execution_results[0].decode::<Amount>()?;
                let balance_b = result.execution_results[1].decode::<Amount>()?;
                let ratio_a = result.execution_results[2].decode::<Amount>()?;
                let ratio_b = result.execution_results[3].decode::<Amount>()?;
                let amount_swapped = amount_b_for_a.value() as f64 * (ratio_a.value() as f64 / 1000.0);
                log::info!(
                    "Swap {n} for {amount_b_for_a} FAUCET -> {amount_swapped} XTR2 @ {ratio_b}:{ratio_a} | pool \
                     liquidity: {balance_a} XTR2 {balance_b} FAUCET",
                    n = (i + 1) * (j + 1) * 2
                );
            }
        }

        Ok(())
    }
}
