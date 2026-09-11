//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashMap, HashSet};

use log::*;
use tari_engine_types::{
    indexed_value::IndexedWellKnownTypes,
    non_fungible::NonFungibleContainer,
    resource::Resource,
    substate::{Substate, SubstateDiff, SubstateId, SubstateValue},
    vault::Vault,
};
use tari_ootle_common_types::optional::Optional;
use tari_ootle_transaction::TransactionId;
use tari_ootle_wallet_sdk::{
    WalletSdk,
    WalletSdkSpec,
    apis::substate::ValidatorScanResult,
    models::{
        AccountChangedEvent,
        AccountCreatedEvent,
        AccountUpdate,
        BalanceChangeSource,
        NewAccountData,
        NonFungibleToken,
        WalletEvent,
    },
};
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib_types::{
    Amount,
    ComponentAddress,
    ConfidentialOutputAddress,
    NonFungibleAddress,
    NonFungibleId,
    ResourceAddress,
    VaultId,
    constants::TOKEN_SYMBOL,
};

use crate::{account_monitor::monitor::AccountMonitorError, notify::Notify};

const LOG_TARGET: &str = "tari::ootle::wallet_services::account_monitor";

#[derive(Debug)]
pub struct AccountScanner<TSpec: WalletSdkSpec> {
    notify: Notify<WalletEvent>,
    wallet_sdk: WalletSdk<TSpec>,
}

impl<TSpec> AccountScanner<TSpec>
where TSpec: WalletSdkSpec
{
    pub fn new(notify: Notify<WalletEvent>, wallet_sdk: WalletSdk<TSpec>) -> Self {
        Self { notify, wallet_sdk }
    }

    pub async fn refresh_account(&self, account_address: ComponentAddress) -> Result<bool, AccountMonitorError> {
        self.refresh_account_with_source(account_address, BalanceChangeSource::Scan)
            .await
    }

    pub(crate) async fn refresh_account_with_source(
        &self,
        account_address: ComponentAddress,
        source: BalanceChangeSource,
    ) -> Result<bool, AccountMonitorError> {
        info!(
            target: LOG_TARGET,
            "🏦 Refreshing account {}", account_address
        );
        let substate_api = self.wallet_sdk.substate_api();
        let accounts_api = self.wallet_sdk.accounts_api();

        let Some(account) = accounts_api.get_account_by_address(&account_address).optional()? else {
            // This is not our account
            return Ok(false);
        };

        let mut is_updated = false;
        let maybe_scan_result = substate_api
            .fetch_substate_from_network(&account_address.into(), None)
            .await
            .optional()?;

        let Some(ValidatorScanResult {
            id: account_substate_id,
            substate: account_value,
        }) = maybe_scan_result
        else {
            if account.is_confirmed_on_chain() {
                warn!(target: LOG_TARGET, "❓️ Account {} does not exist according to indexer but confirmed_on_chain = true", account_address);
            }
            // Otherwise, the account is not on-chain, so we wouldn't expect the indexer to have it

            return Ok(false);
        };

        if !account.is_confirmed_on_chain() {
            // Mark the account as on-chain if it is not already
            self.mark_account_as_on_chain(&account_address)?;
            is_updated = true;
        }

        let indexed_value = IndexedWellKnownTypes::from_value(account_value.component().unwrap().state())?;
        substate_api.save_root(
            account_substate_id.as_versioned_ref(),
            indexed_value.referenced_substates(),
        )?;
        let known_child_vaults = substate_api
            .load_dependent_substates(&[account_substate_id.substate_id()])?
            .into_iter()
            .filter(|s| s.substate_id().is_vault())
            .map(|s| {
                let version = s.version();
                (s.into_substate_id(), version)
            })
            .collect::<HashMap<_, _>>();
        for vault_id in indexed_value.vault_ids() {
            let vault_substate_id = SubstateId::Vault(*vault_id);
            let Some(ValidatorScanResult {
                id: versioned_vault_substate_id,
                substate,
                ..
            }) = substate_api
                .fetch_substate_from_network(&vault_substate_id, None)
                .await
                .optional()?
            else {
                warn!(target: LOG_TARGET, "Vault {} for account {} does not exist according to indexer", vault_substate_id, account_substate_id);
                continue;
            };

            let maybe_vault_version = known_child_vaults.get(&vault_substate_id).copied().flatten();
            if let Some(vault_version) = maybe_vault_version {
                // The first time a vault is found, know about the vault substate from the tx result but never added
                // it to the database.
                if versioned_vault_substate_id.version() <= vault_version && accounts_api.has_vault(vault_id)? {
                    info!(target: LOG_TARGET, "Vault {} is up to date", versioned_vault_substate_id.substate_id());
                    continue;
                }
            }

            let SubstateValue::Vault(latest_vault) = substate else {
                error!(target: LOG_TARGET, "Substate {} is not a vault. This should be impossible.", vault_substate_id);
                continue;
            };

            let Some(account_addr) = account_substate_id.substate_id().as_component_address() else {
                continue;
            };
            self.add_vault_to_account_if_not_exist(&account_addr, *vault_id, &latest_vault)
                .await?;
            self.refresh_vault(
                account_addr,
                *vault_id,
                versioned_vault_substate_id.version(),
                &latest_vault,
                Default::default(),
                source,
            )
            .await?;

            substate_api.save_child(
                account_substate_id.substate_id(),
                versioned_vault_substate_id.as_versioned_ref(),
                [SubstateId::from(*latest_vault.resource_address())],
            )?;
            is_updated = true;

            self.notify.notify(AccountChangedEvent {
                account_address,
                version: account_substate_id.version(),
            });
        }

        Ok(is_updated)
    }

    #[allow(clippy::too_many_lines)]
    async fn refresh_vault(
        &self,
        account_address: ComponentAddress,
        vault_id: VaultId,
        vault_version: u64,
        latest_vault: &Vault,
        updated_nft_data: HashMap<NonFungibleId, NonFungibleContainer>,
        source: BalanceChangeSource,
    ) -> Result<bool, AccountMonitorError> {
        let accounts_api = self.wallet_sdk.accounts_api();

        let new_balance = latest_vault.balance();
        if !accounts_api.exists_by_address(&account_address)? {
            // This is not our account
            return Ok(false);
        }
        let mut has_changed = false;

        let maybe_current_vault = accounts_api.get_vault(&vault_id).optional()?;
        match maybe_current_vault {
            Some(ref vault) => {
                if vault.account_address != account_address {
                    return Err(AccountMonitorError::UnexpectedSubstate(format!(
                        "Vault {} is not in account {}",
                        vault_id, account_address
                    )));
                }
            },
            None => {
                info!(
                    target: LOG_TARGET,
                    "🔒️ NEW vault {} in account {}",
                    vault_id,
                    account_address
                );

                let resource = self.fetch_and_cache_resource(latest_vault.resource_address()).await?;
                let token_symbol = resource.token_symbol().map(|s| s.to_string());
                let divisibility = resource.divisibility();

                accounts_api.add_vault(
                    account_address,
                    vault_id,
                    vault_version,
                    *latest_vault.resource_address(),
                    latest_vault.resource_type(),
                    token_symbol,
                    divisibility,
                )?;
                has_changed = true;
            },
        }

        if let Some(transaction_id) = source.transaction_id() {
            // Claim a scan-recorded change at this vault version for the transaction. A record already
            // existing for this transaction (e.g. its finalize record) must not skip the update below: the
            // balance-change insert merges the dimension this update adds and rejects replayed
            // re-statements, and the vault snapshot comparison makes a full replay a no-op.
            accounts_api.attribute_balance_change_to_transaction(&vault_id, vault_version, transaction_id)?;
        }

        info!(
            target: LOG_TARGET,
            "🔒️ vault {} in account {} has new balance {}",
            vault_id,
            account_address,
            new_balance
        );
        if let Some(commitments) = latest_vault.get_confidential_commitments() {
            info!(
                target: LOG_TARGET,
                "🔒️ vault {} in account {} has {} confidential commitments",
                vault_id,
                account_address,
                commitments.len()
            );
            // Confidential OutputBodies live in separate ConfidentialOutput substates. Only commitments the
            // wallet does not already have a record for are fetched (verify_and_update_confidential_outputs
            // skips known ones anyway), in one batched network request. A commitment the indexer does not
            // return (e.g. spent since the vault was fetched) is skipped rather than failing the refresh.
            let account = self
                .wallet_sdk
                .accounts_api()
                .get_account_by_address(&account_address)?;
            let resource_address = *latest_vault.resource_address();

            let known_commitments = self
                .wallet_sdk
                .confidential_outputs_api()
                .outputs_get_many(&resource_address, Some(&account_address), None)?
                .into_iter()
                .filter(|o| o.vault_id == vault_id)
                .map(|o| o.commitment)
                .collect::<HashSet<_>>();

            let ids_to_fetch = commitments
                .iter()
                .filter(|commitment| !known_commitments.contains(*commitment))
                .map(|commitment| {
                    SubstateId::ConfidentialOutput(ConfidentialOutputAddress::new(resource_address, *commitment))
                })
                .collect::<Vec<_>>();
            let mut substates = self
                .wallet_sdk
                .substate_api()
                .get_substates_from_network(ids_to_fetch)
                .await?;

            let mut outputs = Vec::with_capacity(substates.len());
            for commitment in commitments {
                let id = SubstateId::ConfidentialOutput(ConfidentialOutputAddress::new(resource_address, *commitment));
                let Some(substate) = substates.remove(&id) else {
                    if !known_commitments.contains(commitment) {
                        warn!(
                            target: LOG_TARGET,
                            "Confidential output {} was not returned by the indexer. Skipping.",
                            id
                        );
                    }
                    continue;
                };
                let output = substate
                    .into_substate_value()
                    .into_confidential_output()
                    .ok_or_else(|| {
                        AccountMonitorError::UnexpectedSubstate(format!(
                            "Expected {} to be a confidential output",
                            commitment
                        ))
                    })?;
                outputs.push((*commitment, output.into_output()));
            }
            self.wallet_sdk
                .confidential_outputs_api()
                .verify_and_update_confidential_outputs(
                    account.account(),
                    vault_id,
                    outputs.iter().map(|(c, o)| (c, o)),
                )?;
            has_changed = true;
        }

        if latest_vault.resource_type().is_non_fungible() {
            info!(
                target: LOG_TARGET,
                "🔒️ updating vault {} in account {} which has {} non-fungible(s)",
                vault_id,
                account_address,
                latest_vault.get_non_fungible_ids().len()
            );
            self.update_vault_nfts(vault_id, latest_vault, updated_nft_data).await?;
        }

        // If the vault has revealed stealth tokens, we should also to scan for UTXOs
        if latest_vault.resource_type().is_stealth() {
            self.associate_resource_with_account(&account_address, *latest_vault.resource_address())
                .await?;
        }

        let new_confidential_balance = if latest_vault.resource_type().is_stealth() {
            self.stealth_balance_for_account_resource(&account_address, latest_vault.resource_address())?
        } else {
            let outputs_api = self.wallet_sdk.confidential_outputs_api();
            outputs_api.get_unspent_balance(&vault_id)?
        };

        if accounts_api.update_vault_balance_and_record_change(
            vault_id,
            vault_version,
            new_balance,
            new_confidential_balance,
            source,
        )? {
            info!(
                target: LOG_TARGET,
                "🔒️ balance updated in vault {} in account {}. Balance is {} and confidential balance is {}",
                vault_id,
                account_address,
                new_balance,
                new_confidential_balance
            );
            has_changed = true;
        }

        Ok(has_changed)
    }

    pub(crate) fn record_stealth_balance_change(
        &self,
        account_address: ComponentAddress,
        resource_address: ResourceAddress,
    ) -> Result<bool, AccountMonitorError> {
        let accounts_api = self.wallet_sdk.accounts_api();
        let maybe_vault = accounts_api
            .get_vault_by_resource(&account_address, &resource_address)
            .optional()?;
        if let Some(vault) = maybe_vault.as_ref() &&
            !vault.resource_type.is_stealth()
        {
            debug!(target: LOG_TARGET, "Vault {} for resource {} is not stealth; skipping UTXO balance-change recording", vault.id, resource_address);
            return Ok(false);
        }

        let new_stealth_balance = self.stealth_balance_for_account_resource(&account_address, &resource_address)?;
        let revealed_balance = maybe_vault
            .as_ref()
            .map(|vault| vault.revealed_balance)
            .unwrap_or_default();
        Ok(accounts_api.record_resource_balance_change(
            account_address,
            resource_address,
            revealed_balance,
            new_stealth_balance,
            BalanceChangeSource::Scan,
        )?)
    }

    fn stealth_balance_for_account_resource(
        &self,
        account_address: &ComponentAddress,
        resource_address: &ResourceAddress,
    ) -> Result<Amount, AccountMonitorError> {
        let balance = self
            .wallet_sdk
            .stealth_outputs_api()
            .get_unspent_balance_for_account_resource(account_address, resource_address)?;
        Ok(balance)
    }

    async fn associate_resource_with_account(
        &self,
        account_address: &ComponentAddress,
        resource_address: ResourceAddress,
    ) -> Result<(), AccountMonitorError> {
        let accounts_api = self.wallet_sdk.accounts_api();
        self.ensure_resource_is_cached(&resource_address).await?;
        accounts_api.associate_stealth_resource(account_address, resource_address)?;
        Ok(())
    }

    async fn ensure_resource_is_cached(&self, resource_address: &ResourceAddress) -> Result<(), AccountMonitorError> {
        let resources_api = self.wallet_sdk.resources_api();
        if resources_api.exists(resource_address)? {
            return Ok(());
        }
        debug!(
            target: LOG_TARGET,
            "Resource {} not in local store. Fetching from network.",
            resource_address
        );
        let _resource = self.fetch_and_cache_resource(resource_address).await?;
        Ok(())
    }

    async fn fetch_and_cache_resource(&self, resx_addr: &ResourceAddress) -> Result<Resource, AccountMonitorError> {
        if let Some(resx) = self.wallet_sdk.resources_api().get(resx_addr).optional()? {
            return Ok(resx);
        }

        let substate = self.fetch_substate(&SubstateId::Resource(*resx_addr)).await?;
        let resx = substate.into_substate_value().into_resource().ok_or_else(|| {
            AccountMonitorError::UnexpectedSubstate(format!("Expected {} to be a resource.", resx_addr))
        })?;
        self.wallet_sdk.resources_api().upsert_resource(resx_addr, &resx)?;
        Ok(resx)
    }

    async fn update_vault_nfts(
        &self,
        vault_id: VaultId,
        latest_vault: &Vault,
        mut updated_nft_data_cache: HashMap<NonFungibleId, NonFungibleContainer>,
    ) -> Result<bool, AccountMonitorError> {
        let non_fungibles_api = self.wallet_sdk.non_fungible_api();
        let mut has_changed = false;

        // Existing NFTs
        // TODO: 1_000_000 is arbitrary
        let existing_nft_ids = non_fungibles_api.get_nft_ids_by_vault_id(&vault_id, 1_000_000, 0)?;
        let latest_nft_ids = latest_vault
            .get_non_fungible_ids()
            .iter()
            .cloned()
            .collect::<HashSet<_>>();

        let new_nft_ids = latest_nft_ids
            .difference(&existing_nft_ids)
            .cloned()
            .collect::<Vec<_>>();
        let removed_nft_ids = existing_nft_ids
            .difference(&latest_nft_ids)
            .cloned()
            .collect::<Vec<_>>();

        debug!(
            target: LOG_TARGET,
            "Local vault {} has {} NFTs, and {} NFTs in updated vault, {} new nft(s).",
            vault_id,
            existing_nft_ids.len(),
            latest_vault.get_non_fungible_ids().len(),
            new_nft_ids.len()
        );

        for to_remove in removed_nft_ids {
            info!(
                target: LOG_TARGET,
                "🧸 Updating NFTs: Non-fungible {} has been transferred from vault {}. Removing it from the stored vault.",
                to_remove,
                vault_id
            );
            if non_fungibles_api
                .remove_nft(&vault_id, &to_remove)
                .optional()?
                .is_none()
            {
                warn!(
                    target: LOG_TARGET,
                    "NonFungible ID {} is found by indexer, but not found in the vault.", to_remove
                );
            }
            has_changed = true;
        }

        for id in new_nft_ids {
            info!(
                target: LOG_TARGET,
                "🧸 Updating NFTs: Non-fungible {} has been added to vault {}. Adding it to the stored vault.",
                id,
                vault_id
            );
            let nft = match updated_nft_data_cache.remove(&id) {
                Some(nft) => nft,
                None => {
                    let substate = self
                        .fetch_substate(&NonFungibleAddress::new(*latest_vault.resource_address(), id.clone()).into())
                        .await
                        .optional()?;
                    let Some(substate) = substate else {
                        warn!(target: LOG_TARGET, "Non-fungible {} for vault {} does not exist according to indexer. Unable to update it", id, vault_id);
                        continue;
                    };
                    substate.into_substate_value().into_non_fungible().ok_or_else(|| {
                        AccountMonitorError::UnexpectedSubstate(format!("Expected {} to be a non-fungible token.", id))
                    })?
                },
            };

            let maybe_nft_contents = nft.contents();

            let non_fungible = NonFungibleToken {
                is_burnt: nft.is_burnt(),
                resource_address: *latest_vault.resource_address(),
                vault_id,
                nft_id: id.clone(),
                data: maybe_nft_contents
                    .map(|nft| nft.data().clone())
                    .unwrap_or(tari_bor::Value::Null),
                mutable_data: maybe_nft_contents
                    .map(|nft| nft.mutable_data().clone())
                    .unwrap_or(tari_bor::Value::Null),
            };

            non_fungibles_api.save_nft(&non_fungible)?;
            has_changed = true;
        }

        Ok(has_changed)
    }

    #[allow(clippy::too_many_lines)]
    pub async fn process_result(
        &self,
        tx_id: TransactionId,
        diff: &SubstateDiff,
        new_account_data: Option<NewAccountData>,
    ) -> Result<(), AccountMonitorError> {
        let substate_api = self.wallet_sdk.substate_api();
        let accounts_api = self.wallet_sdk.accounts_api();

        let mut new_account = None;
        if let Some(account) = new_account_data {
            let existing_account = accounts_api.get_account_by_address(&account.address).optional()?;
            // Check that the new account was created in this transaction
            if diff.up_iter().any(|(id, _)| *id == account.address) {
                // NOTE: that account must exist by this point
                self.mark_account_as_on_chain(&account.address)?;
                new_account = existing_account;
                debug!(
                    target: LOG_TARGET,
                    "🏦 New account {} created in transaction {}",
                    account.address,
                    tx_id
                );
            } else if existing_account.is_none_or(|acc| !acc.is_confirmed_on_chain()) {
                warn!(
                    target: LOG_TARGET,
                    "⚠️ Transaction {} does not contain the new account {} but the account does not exist or is not on-chain. This should be impossible.",
                    tx_id,
                    account.address,
                );
                // Continue anyway
            } else {
                info!(
                    target: LOG_TARGET,
                    "🏦 Account {} already exists and on-chain (processing transaction result {})",
                    account.address,
                    tx_id
                );
            }
        }

        let mut vaults = diff
            .up_iter()
            .filter(|(a, _)| a.is_vault())
            .map(|(a, s)| (a.as_vault_id().unwrap(), s))
            .collect::<HashMap<_, _>>();

        let stealth_outputs_api = self.wallet_sdk.stealth_outputs_api();
        let utxos = diff.up_iter().filter(|(id, _)| id.is_utxo()).map(|(id, s)| {
            let utxo = s
                .substate_value()
                .as_utxo()
                .unwrap_or_else(|| panic!("Expected {} to be a UTXO.", id));
            (id.as_utxo_address().expect("is_utxo checked"), utxo)
        });

        let touched_stealth_balances = stealth_outputs_api.verify_and_update_outputs(utxos)?;

        let accounts =
            diff.up_iter()
                .filter(|(_, s)| is_account(s))
                .filter_map(|(a, s)| {
                    match IndexedWellKnownTypes::from_value(s.substate_value().component().unwrap().state()) {
                        Ok(value) => Some((
                            a.as_component_address().expect("BUG: substate id is a component"),
                            value,
                            s.version(),
                        )),
                        Err(e) => {
                            error!(
                                target: LOG_TARGET,
                                "🏦 Failed to parse account substate {} in tx {}: {}", a, tx_id, e
                            );
                            None
                        },
                    }
                });

        let mut updated_accounts = HashSet::new();
        // Find and process all new/existing vaults
        for (account_address, value, account_version) in accounts {
            // If we know about this account, mark it as on-chain (if it isn't already)
            if self.mark_account_as_on_chain(&account_address).optional()?.is_none() {
                debug!(
                    target: LOG_TARGET,
                    "🏦 Account {} in transaction {} does not belong to us.",
                    account_address,
                    tx_id
                );
                continue;
            }
            let mut has_changed = false;
            for vault_id in value.vault_ids() {
                // Any vaults we process here do not need to be reprocesed later
                if let Some(vault_substate) = vaults.remove(vault_id) {
                    let vault_version = vault_substate.version();
                    let Some(vault) = vault_substate.substate_value().vault() else {
                        error!(target: LOG_TARGET, "🏦 Substate {} is not a vault. This should be impossible.", vault_id);
                        continue;
                    };
                    has_changed |= self
                        .add_vault_to_account_if_not_exist(&account_address, *vault_id, vault)
                        .await?;

                    let updated_nfts = vault
                        .get_non_fungible_ids()
                        .iter()
                        .filter_map(|nft_id| {
                            diff.up_iter()
                                .find(|(id, _)| id.as_non_fungible_address().map(|a| a.id()) == Some(nft_id))
                                .map(|(_, s)| {
                                    let nft = s
                                        .substate_value()
                                        .non_fungible()
                                        .unwrap_or_else(|| panic!("Expected {} to be a non-fungible token.", nft_id));
                                    (nft_id.clone(), nft.clone())
                                })
                        })
                        .collect();

                    has_changed |= self
                        .refresh_vault(
                            account_address,
                            *vault_id,
                            vault_version,
                            vault,
                            updated_nfts,
                            BalanceChangeSource::Transaction { transaction_id: tx_id },
                        )
                        .await?;
                }
            }

            if has_changed {
                updated_accounts.insert((account_address, account_version));
            }
        }

        // Process all existing vaults that belong to an account
        for (vault_id, substate) in vaults {
            let vault_addr = SubstateId::Vault(vault_id);
            let SubstateValue::Vault(vault) = substate.substate_value() else {
                error!(target: LOG_TARGET, "🏦 Substate {} is not a vault. This should be impossible.", vault_addr);
                continue;
            };

            // Try and get the account address from the vault
            let maybe_vault_substate = substate_api.get_substate(&vault_addr).optional()?;
            let Some(vault_substate) = maybe_vault_substate else {
                debug!(target: LOG_TARGET, "🏦 Vault {} is not known and therefore does not belong to us.", vault_addr);
                continue;
            };

            let Some(account_addr) = vault_substate.parent_address else {
                // Happens if this is someone else's vault
                debug!(target: LOG_TARGET, "🏦 Vault {} has no parent component.", vault_addr);
                continue;
            };
            let account_addr = account_addr.as_component_address().unwrap_or_else(|| {
                panic!(
                    "BUG: Vault {} has a parent address that is not a component.",
                    vault_addr
                )
            });

            // Check if this vault is associated with an account
            if !accounts_api.exists_by_address(&account_addr)? {
                info!(
                    target: LOG_TARGET,
                    "🏦 Vault {} not in any known account",
                    vault_addr,
                );
                continue;
            };

            // Version unchanged
            let account_version = substate_api
                .get_substate(&account_addr.into())
                .optional()?
                .map(|s| s.substate_id.version())
                .unwrap_or(0);

            self.add_vault_to_account_if_not_exist(&account_addr, vault_id, vault)
                .await?;

            let updated_nfts = vault
                .get_non_fungible_ids()
                .iter()
                .filter_map(|nft_id| {
                    diff.up_iter()
                        .find(|(id, _)| id.as_non_fungible_address().map(|a| a.id()) == Some(nft_id))
                        .map(|(_, s)| {
                            let nft = s
                                .substate_value()
                                .non_fungible()
                                .unwrap_or_else(|| panic!("Expected {} to be a non-fungible token.", nft_id));
                            (nft_id.clone(), nft.clone())
                        })
                })
                .collect();

            // Update the vault balance / confidential outputs
            self.refresh_vault(
                account_addr,
                vault_id,
                substate.version(),
                vault,
                updated_nfts,
                BalanceChangeSource::Transaction { transaction_id: tx_id },
            )
            .await?;
            updated_accounts.insert((account_addr, account_version));
        }

        for (account_address, resource_address) in touched_stealth_balances {
            let revealed_balance = accounts_api
                .get_vault_by_resource(&account_address, &resource_address)
                .optional()?
                .map(|vault| vault.revealed_balance)
                .unwrap_or_default();
            let confidential_balance =
                self.stealth_balance_for_account_resource(&account_address, &resource_address)?;
            if accounts_api.record_resource_balance_change(
                account_address,
                resource_address,
                revealed_balance,
                confidential_balance,
                BalanceChangeSource::Transaction { transaction_id: tx_id },
            )? {
                let account_version = substate_api
                    .get_substate(&account_address.into())
                    .optional()?
                    .map(|s| s.substate_id.version())
                    .unwrap_or(0);
                updated_accounts.insert((account_address, account_version));
            }
        }

        if let Some(account) = new_account {
            debug!(
                target: LOG_TARGET,
                "🏦 Notifying account created for tx {}: {}",
                tx_id,
                account
            );
            self.notify.notify(AccountCreatedEvent {
                account: account.account,
                _created_by_tx: tx_id,
            });
        }
        if !updated_accounts.is_empty() {
            debug!(
                target: LOG_TARGET,
                "🏦 Notifying {} account(s) changed for tx {}",
                updated_accounts.len(),
                tx_id
            );
            for (account_address, version) in updated_accounts {
                self.notify.notify(AccountChangedEvent {
                    account_address,
                    version,
                });
            }
        }

        Ok(())
    }

    async fn fetch_substate(&self, substate_id: &SubstateId) -> Result<Substate, AccountMonitorError> {
        let substate_api = self.wallet_sdk.substate_api();
        let ValidatorScanResult { substate, id: address } =
            substate_api.fetch_substate_from_network(substate_id, None).await?;
        Ok(Substate::new(address.version(), substate))
    }

    fn mark_account_as_on_chain(&self, account_addr: &ComponentAddress) -> Result<(), AccountMonitorError> {
        self.wallet_sdk
            .accounts_api()
            .update_account(account_addr, AccountUpdate {
                is_account_on_chain: Some(true),
                ..Default::default()
            })?;
        Ok(())
    }

    async fn add_vault_to_account_if_not_exist(
        &self,
        account_addr: &ComponentAddress,
        vault_id: VaultId,
        vault: &Vault,
    ) -> Result<bool, AccountMonitorError> {
        let accounts_api = self.wallet_sdk.accounts_api();
        if !accounts_api.exists_by_address(account_addr)? {
            // This is not our account
            return Ok(false);
        }
        if accounts_api.has_vault(&vault_id)? {
            return Ok(false);
        }
        let maybe_resource = match self.fetch_and_cache_resource(vault.resource_address()).await {
            Ok(r) => Some(r),
            Err(e) => {
                warn!(
                    target: LOG_TARGET,
                    "🏦 Failed to scan vault {} from VN: {}",
                    vault_id,
                    e
                );
                None
            },
        };

        let token_symbol = maybe_resource
            .as_ref()
            .and_then(|r| r.metadata().get(TOKEN_SYMBOL).map(|s| s.to_string()));
        let divisibility = maybe_resource.as_ref().map(|r| r.divisibility()).unwrap_or(0);

        info!(
            target: LOG_TARGET,
            "🏦 New {} in account {}",
            vault_id,
            account_addr
        );
        accounts_api.add_vault(
            *account_addr,
            vault_id,
            0,
            *vault.resource_address(),
            vault.resource_type(),
            token_symbol,
            divisibility,
        )?;

        Ok(true)
    }
}

fn is_account(s: &Substate) -> bool {
    s.substate_value()
        .component()
        .filter(|c| *c.template_address() == ACCOUNT_TEMPLATE_ADDRESS)
        .is_some()
}
