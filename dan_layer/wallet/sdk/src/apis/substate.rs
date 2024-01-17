//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use log::*;
use tari_dan_common_types::optional::{IsNotFoundError, Optional};
use tari_engine_types::{
    indexed_value::{IndexedValueError, IndexedWellKnownTypes},
    substate::{SubstateAddress, SubstateValue},
    transaction_receipt::TransactionReceiptAddress,
};
use tari_transaction::TransactionId;

use crate::{
    models::{SubstateModel, VersionedSubstateAddress},
    network::WalletNetworkInterface,
    storage::{WalletStorageError, WalletStore, WalletStoreReader, WalletStoreWriter},
};

const LOG_TARGET: &str = "tari::dan::wallet_sdk::apis::substate";

pub struct SubstatesApi<'a, TStore, TNetworkInterface> {
    store: &'a TStore,
    network_interface: &'a TNetworkInterface,
}

impl<'a, TStore, TNetworkInterface> SubstatesApi<'a, TStore, TNetworkInterface>
where
    TStore: WalletStore,
    TNetworkInterface: WalletNetworkInterface,
    TNetworkInterface::Error: IsNotFoundError,
{
    pub fn new(store: &'a TStore, network_interface: &'a TNetworkInterface) -> Self {
        Self {
            store,
            network_interface,
        }
    }

    pub fn get_substate(&self, address: &SubstateAddress) -> Result<SubstateModel, SubstateApiError> {
        let mut tx = self.store.create_read_tx()?;
        let substate = tx.substates_get(address)?;
        Ok(substate)
    }

    pub fn load_dependent_substates(
        &self,
        parents: &[&SubstateAddress],
    ) -> Result<Vec<VersionedSubstateAddress>, SubstateApiError> {
        let mut substate_addresses = Vec::with_capacity(parents.len());
        let mut tx = self.store.create_read_tx()?;
        // TODO: Could be optimised, also perhaps we need to traverse all ancestors recursively
        for parent_addr in parents {
            let parent = tx.substates_get(parent_addr)?;
            let children = tx.substates_get_children(parent_addr)?;
            substate_addresses.push(parent.address);
            substate_addresses.extend(children.into_iter().map(|s| s.address));
        }
        Ok(substate_addresses)
    }

    pub async fn locate_dependent_substates(
        &self,
        parents: &[SubstateAddress],
    ) -> Result<Vec<VersionedSubstateAddress>, SubstateApiError> {
        let mut substate_addresses = HashMap::with_capacity(parents.len());

        for parent_addr in parents {
            match self.store.with_read_tx(|tx| tx.substates_get(parent_addr)).optional()? {
                Some(parent) => {
                    substate_addresses.insert(parent.address.address, parent.address.version);
                    let children = self.store.with_read_tx(|tx| tx.substates_get_children(parent_addr))?;
                    substate_addresses.extend(children.into_iter().map(|s| (s.address.address, s.address.version)));
                },
                None => {
                    let ValidatorScanResult { address, substate, .. } =
                        self.scan_for_substate(parent_addr, None).await?;
                    substate_addresses.insert(address.address.clone(), address.version);

                    if substate_addresses.contains_key(&address.address) {
                        continue;
                    }

                    match substate {
                        SubstateValue::Component(data) => {
                            let value = IndexedWellKnownTypes::from_value(&data.body.state)?;
                            for addr in value.referenced_substates() {
                                if substate_addresses.contains_key(&addr) {
                                    continue;
                                }

                                let ValidatorScanResult { address: addr, .. } =
                                    self.scan_for_substate(&addr, None).await?;
                                substate_addresses.insert(addr.address, addr.version);
                            }
                        },
                        SubstateValue::Resource(_) => {},
                        SubstateValue::TransactionReceipt(tx_receipt) => {
                            let tx_receipt_addr = SubstateAddress::TransactionReceipt(TransactionReceiptAddress::new(
                                tx_receipt.transaction_hash,
                            ));
                            if substate_addresses.contains_key(&tx_receipt_addr) {
                                continue;
                            }
                            let ValidatorScanResult { address: addr, .. } =
                                self.scan_for_substate(&tx_receipt_addr, None).await?;
                            substate_addresses.insert(addr.address, addr.version);
                        },
                        SubstateValue::Vault(vault) => {
                            let resx_addr = SubstateAddress::Resource(*vault.resource_address());
                            if substate_addresses.contains_key(&resx_addr) {
                                continue;
                            }
                            let ValidatorScanResult { address: addr, .. } =
                                self.scan_for_substate(&resx_addr, None).await?;
                            substate_addresses.insert(addr.address, addr.version);
                        },
                        SubstateValue::NonFungible(_) => {},
                        SubstateValue::NonFungibleIndex(addr) => {
                            let resx_addr = SubstateAddress::Resource(*addr.referenced_address().resource_address());
                            if substate_addresses.contains_key(&resx_addr) {
                                continue;
                            }
                            let ValidatorScanResult { address: addr, .. } =
                                self.scan_for_substate(&resx_addr, None).await?;
                            substate_addresses.insert(addr.address, addr.version);
                        },
                        SubstateValue::UnclaimedConfidentialOutput(_) => {},
                        SubstateValue::FeeClaim(_) => {},
                    }
                },
            }
        }

        Ok(substate_addresses
            .into_iter()
            .map(|(address, version)| VersionedSubstateAddress { address, version })
            .collect())
    }

    pub async fn scan_for_substate(
        &self,
        address: &SubstateAddress,
        version_hint: Option<u32>,
    ) -> Result<ValidatorScanResult, SubstateApiError> {
        debug!(
            target: LOG_TARGET,
            "Scanning for substate {} at version {}",
            address,
            version_hint.unwrap_or(0)
        );

        // TODO: make configuration option to not do network requests
        let resp = self
            .network_interface
            .query_substate(address, version_hint, true)
            .await
            .optional()
            .map_err(|e| SubstateApiError::NetworkIndexerError(e.into()))?
            .ok_or_else(|| SubstateApiError::SubstateDoesNotExist {
                address: address.clone(),
            })?;

        debug!(
            target: LOG_TARGET,
            "Found substate {} at version {}", address, resp.version
        );
        Ok(ValidatorScanResult {
            address: VersionedSubstateAddress {
                address: address.clone(),
                version: resp.version,
            },
            created_by_tx: resp.created_by_transaction,
            substate: resp.substate.into_substate_value(),
        })
    }

    pub fn save_root(
        &self,
        created_by_tx: TransactionId,
        address: VersionedSubstateAddress,
    ) -> Result<(), SubstateApiError> {
        self.store.with_write_tx(|tx| {
            let maybe_removed = tx.substates_remove(&address.address).optional()?;
            tx.substates_insert_root(
                created_by_tx,
                address,
                maybe_removed.as_ref().and_then(|s| s.module_name.clone()),
                maybe_removed.and_then(|s| s.template_address),
            )
        })?;
        Ok(())
    }

    pub fn save_child(
        &self,
        created_by_tx: TransactionId,
        parent: SubstateAddress,
        child: VersionedSubstateAddress,
    ) -> Result<(), SubstateApiError> {
        self.store.with_write_tx(|tx| {
            tx.substates_remove(&child.address).optional()?;
            tx.substates_insert_child(created_by_tx, parent, child)
        })?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SubstateApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
    #[error("Network network_interface error: {0}")]
    NetworkIndexerError(anyhow::Error),
    #[error("Invalid validator node response: {0}")]
    InvalidValidatorNodeResponse(String),
    #[error("Substate {address} does not exist")]
    SubstateDoesNotExist { address: SubstateAddress },
    #[error("ValueVisitorError: {0}")]
    ValueVisitorError(#[from] IndexedValueError),
}

impl IsNotFoundError for SubstateApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::SubstateDoesNotExist { .. }) ||
            matches!(self, Self::StoreError(e) if e.is_not_found_error())
    }
}

pub struct ValidatorScanResult {
    pub address: VersionedSubstateAddress,
    pub created_by_tx: TransactionId,
    pub substate: SubstateValue,
}
