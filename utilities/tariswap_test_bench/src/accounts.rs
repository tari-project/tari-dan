//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::RangeInclusive;

use tari_crypto::{keys::PublicKey as _, ristretto::RistrettoPublicKey};
use tari_dan_common_types::SubstateRequirement;
use tari_dan_wallet_sdk::{apis::key_manager::TRANSACTION_BRANCH, models::Account};
use tari_engine_types::{component::new_component_address_from_public_key, indexed_value::IndexedWellKnownTypes};
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::{
    args,
    constants::{XTR, XTR_FAUCET_COMPONENT_ADDRESS, XTR_FAUCET_VAULT_ADDRESS},
    models::Amount,
    resource::ResourceType,
};
use tari_transaction::Transaction;

use crate::{faucet::Faucet, runner::Runner};

impl Runner {
    pub async fn create_account_with_free_coins(&mut self) -> anyhow::Result<Account> {
        let key = self.sdk.key_manager_api().derive_key(TRANSACTION_BRANCH, 0)?;
        let owner_public_key = RistrettoPublicKey::from_secret_key(&key.key);

        let account_address = new_component_address_from_public_key(&ACCOUNT_TEMPLATE_ADDRESS, &owner_public_key);

        let transaction = Transaction::builder()
            .with_fee_instructions_builder(|builder| {
                builder
                    .call_method(XTR_FAUCET_COMPONENT_ADDRESS, "take", args![Amount(1_000_000_000)])
                    .put_last_instruction_output_on_workspace("coins")
                    .create_account_with_bucket(owner_public_key, "coins")
                    .call_method(account_address, "pay_fee", args![Amount(1000)])
            })
            .with_inputs([
                SubstateRequirement::unversioned(XTR_FAUCET_COMPONENT_ADDRESS),
                SubstateRequirement::unversioned(XTR_FAUCET_VAULT_ADDRESS),
            ])
            .sign(&key.key)
            .build();

        let finalize = self.submit_transaction_and_wait(transaction).await?;
        let diff = finalize.result.accept().unwrap();
        let (account, _) = diff.up_iter().find(|(addr, _)| addr.is_component()).unwrap();
        let (vault, _) = diff
            .up_iter()
            .find(|(addr, _)| *addr != XTR_FAUCET_VAULT_ADDRESS && addr.is_vault())
            .unwrap();

        self.sdk.accounts_api().add_account(None, account, 0, true)?;
        self.sdk.accounts_api().add_vault(
            account.clone(),
            vault.clone(),
            XTR,
            ResourceType::Confidential,
            Some("XTR".to_string()),
        )?;
        let account = self.sdk.accounts_api().get_account_by_address(account)?;

        Ok(account)
    }

    pub async fn create_accounts(
        &mut self,
        pay_fee_account: &Account,
        account_key_indexes: RangeInclusive<u64>,
    ) -> anyhow::Result<Vec<Account>> {
        let key = self.sdk.key_manager_api().derive_key(TRANSACTION_BRANCH, 0)?;
        let key_index_start = *account_key_indexes.start();
        let num_accounts = *account_key_indexes.end() as usize - key_index_start as usize + 1;
        let owners = account_key_indexes
            .map(|idx| {
                let key = self.sdk.key_manager_api().derive_key(TRANSACTION_BRANCH, idx)?;
                Ok(key)
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        let mut builder = Transaction::builder().fee_transaction_pay_from_component(
            pay_fee_account.address.as_component_address().unwrap(),
            Amount(1000 * owners.len() as i64),
        );
        for owner in &owners {
            builder = builder.create_account(RistrettoPublicKey::from_secret_key(&owner.key));
        }

        let pay_fee_vault = self
            .sdk
            .accounts_api()
            .get_vault_by_resource(&pay_fee_account.address, &XTR)?;

        let transaction = builder
            .with_inputs([
                SubstateRequirement::unversioned(pay_fee_account.address.clone()),
                SubstateRequirement::unversioned(pay_fee_vault.address),
                SubstateRequirement::unversioned(pay_fee_vault.resource_address),
            ])
            .sign(&key.key)
            .build();

        let finalize = self.submit_transaction_and_wait(transaction).await?;
        let diff = finalize.result.accept().unwrap();
        let mut accounts = Vec::with_capacity(num_accounts);

        for owner in owners {
            let account_addr = diff
                .up_iter()
                .map(|(addr, _)| addr)
                .filter(|addr| addr.is_component())
                .filter(|addr| **addr != pay_fee_account.address)
                .find(|addr| {
                    new_component_address_from_public_key(
                        &ACCOUNT_TEMPLATE_ADDRESS,
                        &RistrettoPublicKey::from_secret_key(&owner.key),
                    ) == **addr
                })
                .expect("New account not found in diff");

            self.sdk
                .accounts_api()
                .add_account(None, account_addr, owner.key_index, false)?;
            let account = self.sdk.accounts_api().get_account_by_address(account_addr)?;
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
        let fee_vault = self
            .sdk
            .accounts_api()
            .get_vault_by_resource(&fee_account.address, &XTR)?;
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
                .call_method(XTR_FAUCET_COMPONENT_ADDRESS, "take", args![Amount(1_000_000)])
                .put_last_instruction_output_on_workspace("xtr")
                .call_method(account.address.as_component_address().unwrap(), "deposit", args![
                    Workspace("xtr")
                ])
                .add_input(SubstateRequirement::unversioned(account.address.clone()));
        }

        let transaction = builder
            .with_inputs([
                SubstateRequirement::unversioned(XTR),
                SubstateRequirement::unversioned(XTR_FAUCET_COMPONENT_ADDRESS),
                SubstateRequirement::unversioned(XTR_FAUCET_VAULT_ADDRESS),
                SubstateRequirement::unversioned(faucet.component_address),
                SubstateRequirement::unversioned(faucet.resource_address),
                SubstateRequirement::unversioned(faucet.vault_address),
                SubstateRequirement::unversioned(fee_vault.account_address),
                SubstateRequirement::unversioned(fee_vault.address),
            ])
            .sign(&key.key)
            .build();

        let result = self.submit_transaction_and_wait(transaction).await?;

        let accounts_and_state = result
            .result
            .accept()
            .unwrap()
            .up_iter()
            .filter(|(addr, _)| {
                *addr != XTR_FAUCET_COMPONENT_ADDRESS &&
                    *addr != faucet.component_address &&
                    *addr != fee_account.address
            })
            .filter_map(|(addr, substate)| Some((addr, substate.substate_value().component()?)))
            .map(|(addr, component)| (addr, IndexedWellKnownTypes::from_value(&component.body.state).unwrap()));

        for (account, indexed) in accounts_and_state {
            for vault_id in indexed.vault_ids() {
                log::info!("Adding vault {} to account {}", vault_id, account);
                self.sdk.accounts_api().add_vault(
                    account.clone(),
                    (*vault_id).into(),
                    faucet.resource_address,
                    ResourceType::Fungible,
                    Some("FAUCET".to_string()),
                )?;
            }
        }

        Ok(())
    }
}
