//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use log::info;
use tari_dan_common_types::{optional::Optional, SubstateRequirement};
use tari_dan_wallet_sdk::{apis::key_manager::TRANSACTION_BRANCH, models::Account};
use tari_engine_types::indexed_value::decode_value_at_path;
use tari_template_lib::{
    args,
    models::{Amount, ComponentAddress, VaultId},
    prelude::{ResourceAddress, ResourceType, XTR},
};
use tari_transaction::Transaction;

use crate::{faucet::Faucet, runner::Runner};

pub struct TariSwap {
    pub component_address: ComponentAddress,
    pub vaults: HashMap<ResourceAddress, VaultId>,
    pub lp_resource_address: ResourceAddress,
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
                XTR,
                faucet.resource_address,
                Amount(1)
            ]);
        }

        let fee_vault = self
            .sdk
            .accounts_api()
            .get_vault_by_resource(&in_account.address, &XTR)?;

        let transaction = builder
            .with_inputs([
                SubstateRequirement::unversioned(in_account.address.clone()),
                SubstateRequirement::unversioned(fee_vault.address.clone()),
                SubstateRequirement::unversioned(XTR),
                SubstateRequirement::unversioned(faucet.resource_address),
            ])
            .sign(&key.key)
            .build();

        let finalize = self.submit_transaction_and_wait(transaction).await?;
        let diff = finalize.result.accept().unwrap();
        Ok(diff
            .up_iter()
            .filter_map(|(addr, value)| {
                let addr = addr
                    .as_component_address()
                    .filter(|_| value.substate_value().component().unwrap().module_name == "TariSwapPool")?;
                let vaults = decode_value_at_path(value.substate_value().component().unwrap().state(), "$.pools")
                    .unwrap()
                    .unwrap();
                let lp_resource_address =
                    decode_value_at_path(value.substate_value().component().unwrap().state(), "$.lp_resource")
                        .unwrap()
                        .unwrap();
                Some(TariSwap {
                    component_address: addr,
                    vaults,
                    lp_resource_address,
                })
            })
            .collect())
    }

    pub async fn add_liquidity(
        &mut self,
        tariswaps: &[TariSwap],
        primary_account: &Account,
        accounts: &[Account],
        amount_a: Amount,
        amount_b: Amount,
        faucet: &Faucet,
    ) -> anyhow::Result<()> {
        let primary_account_key = self
            .sdk
            .key_manager_api()
            .derive_key(TRANSACTION_BRANCH, primary_account.key_index)?;
        let mut tx_ids = Vec::with_capacity(200);

        for i in 0..5 {
            for (i, tariswap) in tariswaps.iter().enumerate().skip(i * 200).take(200) {
                let account = &accounts[i % accounts.len()];
                let key = self
                    .sdk
                    .key_manager_api()
                    .derive_key(TRANSACTION_BRANCH, account.key_index)?;
                let xtr_vault = self.sdk.accounts_api().get_vault_by_resource(&account.address, &XTR)?;
                let faucet_vault = self
                    .sdk
                    .accounts_api()
                    .get_vault_by_resource(&account.address, &faucet.resource_address)?;
                let maybe_lp_vault = self
                    .sdk
                    .accounts_api()
                    .get_vault_by_resource(&account.address, &tariswap.lp_resource_address)
                    .optional()?;

                let transaction = Transaction::builder()
                    .with_inputs(maybe_lp_vault.map(|v| SubstateRequirement::unversioned(v.address)))
                    .with_inputs([
                        SubstateRequirement::unversioned(account.address.clone()),
                        SubstateRequirement::unversioned(xtr_vault.address),
                        SubstateRequirement::unversioned(faucet_vault.address),
                        SubstateRequirement::unversioned(tariswap.component_address),
                        SubstateRequirement::unversioned(faucet.resource_address),
                        SubstateRequirement::unversioned(XTR),
                    ])
                    .with_inputs(tariswap.vaults.values().map(|v| SubstateRequirement::unversioned(*v)))
                    .fee_transaction_pay_from_component(account.address.as_component_address().unwrap(), Amount(1000))
                    .call_method(account.address.as_component_address().unwrap(), "withdraw", args![
                        XTR, amount_a
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
                    .sign(&primary_account_key.key)
                    .sign(&key.key)
                    .build();

                tx_ids.push((account.address.clone(), self.submit_transaction(transaction).await?));
            }

            for (account, tx_id) in tx_ids.drain(..) {
                let result = self.wait_for_transaction(tx_id).await?;
                let diff = result.result.accept().unwrap();
                let lp_vault = diff
                    .up_iter()
                    .find_map(|(addr, s)| {
                        let addr = addr.as_vault_id()?;
                        if *s.substate_value().vault().unwrap().resource_address() == tariswaps[0].lp_resource_address {
                            Some(addr)
                        } else {
                            None
                        }
                    })
                    .ok_or_else(|| anyhow::anyhow!("LP Vault not found"))?;
                self.sdk.accounts_api().add_vault(
                    account,
                    lp_vault.into(),
                    tariswaps[0].lp_resource_address,
                    ResourceType::NonFungible,
                    Some("LP".to_string()),
                )?;
            }
            info!("⏳️ Added liquidity to pools {}-{}", i * 200, (i + 1) * 200);
        }

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    pub async fn do_tariswap_swaps(
        &mut self,
        tariswaps: &[TariSwap],
        primary_account: &Account,
        accounts: &[Account],
        amount_a_for_b: Amount,
        amount_b_for_a: Amount,
        faucet: &Faucet,
    ) -> anyhow::Result<()> {
        let primary_account_key = self
            .sdk
            .key_manager_api()
            .derive_key(TRANSACTION_BRANCH, primary_account.key_index)?;

        let mut tx_ids = vec![];
        // Swap XTR for faucet
        for i in 0..5 {
            for (i, account) in accounts.iter().enumerate().skip(i * 200).take(200) {
                let tariswap = &tariswaps[i % tariswaps.len()];
                let key = self
                    .sdk
                    .key_manager_api()
                    .derive_key(TRANSACTION_BRANCH, account.key_index)?;
                let xtr_vault = self.sdk.accounts_api().get_vault_by_resource(&account.address, &XTR)?;
                let faucet_vault = self
                    .sdk
                    .accounts_api()
                    .get_vault_by_resource(&account.address, &faucet.resource_address)?;
                let maybe_lp_vault = self
                    .sdk
                    .accounts_api()
                    .get_vault_by_resource(&account.address, &tariswap.lp_resource_address)
                    .optional()?;
                let transaction = Transaction::builder()
                    .with_inputs(maybe_lp_vault.map(|v| SubstateRequirement::unversioned(v.address)))
                    .with_inputs([
                        SubstateRequirement::unversioned(account.address.clone()),
                        SubstateRequirement::unversioned(xtr_vault.address),
                        SubstateRequirement::unversioned(faucet_vault.address),
                        SubstateRequirement::unversioned(tariswap.component_address),
                        SubstateRequirement::unversioned(faucet.resource_address),
                        SubstateRequirement::unversioned(XTR),
                        SubstateRequirement::unversioned(tariswap.lp_resource_address),
                    ])
                    .with_inputs(tariswap.vaults.values().map(|v| SubstateRequirement::unversioned(*v)))
                    .fee_transaction_pay_from_component(account.address.as_component_address().unwrap(), Amount(1000))
                    .call_method(tariswap.component_address, "get_pool_balance", args![XTR,])
                    .call_method(tariswap.component_address, "get_pool_balance", args![
                        faucet.resource_address,
                    ])
                    .call_method(tariswap.component_address, "get_pool_ratio", args![XTR, Amount(1000)])
                    .call_method(tariswap.component_address, "get_pool_ratio", args![
                        faucet.resource_address,
                        Amount(1000)
                    ])
                    .call_method(account.address.as_component_address().unwrap(), "withdraw", args![
                        XTR,
                        amount_a_for_b
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
                    .sign(&primary_account_key.key)
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
                info!(
                    "Swap {n} for {amount_a_for_b} XTR -> {amount_swapped} FAUCET @ {ratio_a}:{ratio_b} | pool \
                     liquidity: {balance_a} XTR {balance_b} FAUCET",
                    n = (i + 1) * (j + 1)
                );
            }
        }

        // Swap faucet for XTR
        for i in 0..5 {
            for (i, account) in accounts.iter().enumerate().skip(i * 200).take(200) {
                let key = self
                    .sdk
                    .key_manager_api()
                    .derive_key(TRANSACTION_BRANCH, account.key_index)?;
                let xtr_vault = self.sdk.accounts_api().get_vault_by_resource(&account.address, &XTR)?;
                let faucet_vault = self
                    .sdk
                    .accounts_api()
                    .get_vault_by_resource(&account.address, &faucet.resource_address)?;
                let tariswap = &tariswaps[i % tariswaps.len()];
                let transaction = Transaction::builder()
                    .with_inputs([
                        SubstateRequirement::unversioned(account.address.clone()),
                        SubstateRequirement::unversioned(xtr_vault.address),
                        SubstateRequirement::unversioned(faucet_vault.address),
                        SubstateRequirement::unversioned(tariswap.component_address),
                        SubstateRequirement::unversioned(faucet.resource_address),
                        SubstateRequirement::unversioned(XTR),
                        SubstateRequirement::unversioned(tariswap.lp_resource_address),
                    ])
                    .with_inputs(tariswap.vaults.values().map(|v| SubstateRequirement::unversioned(*v)))
                    .fee_transaction_pay_from_component(account.address.as_component_address().unwrap(), Amount(1000))
                    .call_method(tariswap.component_address, "get_pool_balance", args![XTR])
                    .call_method(tariswap.component_address, "get_pool_balance", args![
                        faucet.resource_address
                    ])
                    .call_method(tariswap.component_address, "get_pool_ratio", args![XTR, Amount(1000)])
                    .call_method(tariswap.component_address, "get_pool_ratio", args![
                        faucet.resource_address,
                        Amount(1000)
                    ])
                    .call_method(account.address.as_component_address().unwrap(), "withdraw", args![
                        faucet.resource_address,
                        amount_b_for_a
                    ])
                    .put_last_instruction_output_on_workspace("b")
                    .call_method(tariswap.component_address, "swap", args![Workspace("b"), XTR,])
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
                    "Swap {n} for {amount_b_for_a} FAUCET -> {amount_swapped} XTR @ {ratio_b}:{ratio_a} | pool \
                     liquidity: {balance_a} XTR {balance_b} FAUCET",
                    n = (i + 1) * (j + 1) * 2
                );
            }
        }

        Ok(())
    }
}
