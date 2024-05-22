//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::RangeInclusive;

use tari_crypto::{keys::PublicKey as _, ristretto::RistrettoPublicKey};
use tari_dan_wallet_sdk::{apis::key_manager::TRANSACTION_BRANCH, models::Account};
use tari_engine_types::component::new_account_address_from_parts;
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::{args, models::Amount};
use tari_transaction::{Instruction, Transaction};

use crate::{faucet::Faucet, runner::Runner};

impl Runner {
    pub async fn create_account_with_free_coins(&mut self) -> anyhow::Result<Account> {
        let key = self.sdk.key_manager_api().derive_key(TRANSACTION_BRANCH, 0)?;
        let owner_public_key = RistrettoPublicKey::from_secret_key(&key.key);

        let account_address = new_account_address_from_parts(&ACCOUNT_TEMPLATE_ADDRESS, &owner_public_key);

        let transaction = Transaction::builder()
            .with_fee_instructions_builder(|builder| {
                builder
                    .add_instruction(Instruction::CreateFreeTestCoins {
                        revealed_amount: 1_000_000_000.into(),
                        output: None,
                    })
                    .put_last_instruction_output_on_workspace("coins")
                    .create_account_with_bucket(owner_public_key, "coins")
                    .call_method(account_address, "pay_fee", args![Amount(1000)])
            })
            .sign(&key.key)
            .build();

        let finalize = self.submit_transaction_and_wait(transaction).await?;
        let diff = finalize.result.accept().unwrap();
        let (account, _) = diff.up_iter().find(|(addr, _)| addr.is_component()).unwrap();

        self.sdk.accounts_api().add_account(None, account, 0, true)?;
        let account = self.sdk.accounts_api().get_account_by_address(account)?;

        Ok(account)
    }

    pub async fn create_accounts(
        &mut self,
        pay_fee_account: &Account,
        account_key_indexes: RangeInclusive<u64>,
    ) -> anyhow::Result<Vec<Account>> {
        let key = self.sdk.key_manager_api().derive_key(TRANSACTION_BRANCH, 0)?;
        let num_accounts = *account_key_indexes.end() as usize - *account_key_indexes.start() as usize + 1;
        let owners = account_key_indexes
            .map(|idx| {
                let key = self.sdk.key_manager_api().derive_key(TRANSACTION_BRANCH, idx)?;
                Ok(RistrettoPublicKey::from_secret_key(&key.key))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        let mut builder = Transaction::builder().fee_transaction_pay_from_component(
            pay_fee_account.address.as_component_address().unwrap(),
            Amount(1000 * owners.len() as i64),
        );
        for owner in owners {
            builder = builder.create_account(owner);
        }
        let transaction = builder.sign(&key.key).build();

        let finalize = self.submit_transaction_and_wait(transaction).await?;
        let diff = finalize.result.accept().unwrap();
        let mut accounts = Vec::with_capacity(num_accounts);

        for (i, account) in diff
            .up_iter()
            .map(|(addr, _)| addr)
            .filter(|addr| addr.is_component())
            .enumerate()
        {
            if *account == pay_fee_account.address {
                continue;
            }
            // TODO: Key index doesnt match
            self.sdk
                .accounts_api()
                .add_account(None, account, i as u64 + 1, false)?;
            let account = self.sdk.accounts_api().get_account_by_address(account)?;
            accounts.push(account);
        }

        Ok(accounts)
    }

    pub async fn fund_accounts(
        &mut self,
        faucet: &Faucet,
        fee_account: &Account,
        accounts: &[Account],
    ) -> anyhow::Result<()> {
        let key = self.sdk.key_manager_api().derive_key(TRANSACTION_BRANCH, 0)?;
        let mut builder = Transaction::builder().fee_transaction_pay_from_component(
            fee_account.address.as_component_address().unwrap(),
            Amount(1000 * accounts.len() as i64),
        );
        for account in accounts {
            builder = builder
                .call_method(faucet.component_address, "take_free_coins", args![])
                .put_last_instruction_output_on_workspace("faucet")
                .call_method(account.address.as_component_address().unwrap(), "deposit", args![
                    Workspace("faucet")
                ])
                .add_instruction(Instruction::CreateFreeTestCoins {
                    revealed_amount: 1_000_000.into(),
                    output: None,
                })
                .put_last_instruction_output_on_workspace("xtr")
                .call_method(account.address.as_component_address().unwrap(), "deposit", args![
                    Workspace("xtr")
                ]);
        }

        let transaction = builder.sign(&key.key).build();

        self.submit_transaction_and_wait(transaction).await?;

        Ok(())
    }
}
