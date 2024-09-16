//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::info;
use tari_dan_common_types::SubstateRequirement;
use tari_dan_wallet_sdk::{apis::key_manager::TRANSACTION_BRANCH, models::Account};
use tari_template_lib::{
    args,
    constants::XTR,
    models::{Amount, ComponentAddress, ResourceAddress, VaultId},
};
use tari_transaction::Transaction;

use crate::runner::Runner;

pub struct Faucet {
    pub component_address: ComponentAddress,
    pub resource_address: ResourceAddress,
    pub vault_address: VaultId,
}

impl Runner {
    pub async fn create_faucet(&mut self, in_account: &Account) -> anyhow::Result<Faucet> {
        let key = self.sdk.key_manager_api().derive_key(TRANSACTION_BRANCH, 0)?;

        let fee_vault = self
            .sdk
            .accounts_api()
            .get_vault_by_resource(&in_account.address, &XTR)?;

        let transaction = Transaction::builder()
            .fee_transaction_pay_from_component(in_account.address.as_component_address().unwrap(), Amount(1000))
            .call_function(self.faucet_template.address, "mint", args![Amount(1_000_000_000)])
            .with_inputs([
                SubstateRequirement::unversioned(in_account.address.clone()),
                SubstateRequirement::unversioned(fee_vault.address.clone()),
                SubstateRequirement::unversioned(fee_vault.resource_address),
            ])
            .sign(&key.key)
            .build();

        let finalize = self.submit_transaction_and_wait(transaction).await?;
        let diff = finalize.result.accept().unwrap();

        let component_address = diff
            .up_iter()
            .find_map(|(addr, s)| {
                addr.as_component_address().filter(|_| {
                    s.substate_value().component().unwrap().template_address == self.faucet_template.address
                })
            })
            .ok_or_else(|| anyhow::anyhow!("Faucet Component address not found"))?;
        let resource_address = diff
            .up_iter()
            .filter(|(addr, _)| *addr != XTR)
            .find_map(|(addr, _)| addr.as_resource_address())
            .ok_or_else(|| anyhow::anyhow!("Faucet Resource address not found"))?;
        let vault_address = diff
            .up_iter()
            .filter_map(|(addr, _)| addr.as_vault_id())
            .find(|addr| *addr != fee_vault.address)
            .ok_or_else(|| anyhow::anyhow!("Faucet Resource address not found"))?;

        info!("✅ Faucet {component_address} created with {resource_address} and {vault_address}");

        Ok(Faucet {
            component_address,
            resource_address,
            vault_address,
        })
    }
}
