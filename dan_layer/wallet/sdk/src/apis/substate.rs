//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::optional::{IsNotFoundError, Optional};
use tari_engine_types::substate::{SubstateAddress, SubstateValue};
use tari_validator_node_client::{
    types::{GetSubstateRequest, SubstateStatus},
    ValidatorNodeClient,
    ValidatorNodeClientError,
};

use crate::{
    models::{SubstateModel, VersionedSubstateAddress},
    storage::{WalletStorageError, WalletStore, WalletStoreReader},
};

pub struct SubstatesApi<'a, TStore> {
    store: &'a TStore,
    validator_node_jrpc_endpoint: &'a str,
}

impl<'a, TStore: WalletStore> SubstatesApi<'a, TStore> {
    pub fn new(store: &'a TStore, validator_node_jrpc_endpoint: &'a str) -> Self {
        Self {
            store,
            validator_node_jrpc_endpoint,
        }
    }

    pub fn get_substate(&self, address: &SubstateAddress) -> Result<SubstateModel, SubstateApiError> {
        let mut tx = self.store.create_read_tx()?;
        let substate = tx.substates_get(address)?;
        Ok(substate)
    }

    pub fn get_substate(&self, address: &SubstateAddress) -> Result<SubstateRecord, SubstateApiError> {
        let mut tx = self.store.create_read_tx()?;
        let substate = tx.substates_get(address)?;
        Ok(substate)
    }

    pub fn load_dependent_substates(
        &self,
        parents: &[SubstateAddress],
    ) -> Result<Vec<VersionedSubstateAddress>, SubstateApiError> {
        let mut substate_addresses = Vec::with_capacity(parents.len());
        let mut tx = self.store.create_read_tx()?;
        // TODO: Could be optimised, also perhaps we need to traverse all ancestors
        for parent_addr in parents {
            let parent = tx.substates_get(parent_addr)?;
            let children = tx.substates_get_children(parent_addr)?;
            substate_addresses.push(parent.address);
            substate_addresses.extend(children.into_iter().map(|s| s.address));
        }
        Ok(substate_addresses)
    }

    pub async fn scan_from_vn(
        &self,
        address: &SubstateAddress,
    ) -> Result<(VersionedSubstateAddress, SubstateValue), SubstateApiError> {
        let mut client = self.connect_validator_node()?;
        let existing = self.store.with_read_tx(|tx| tx.substates_get(address)).optional()?;
        let mut version = existing.map(|s| s.address.version).unwrap_or(0);

        loop {
            let resp = client
                .get_substate(GetSubstateRequest {
                    address: address.clone(),
                    version,
                })
                .await?;

            let status = resp.status;
            if let Some(value) = resp.value {
                return Ok((
                    VersionedSubstateAddress {
                        address: address.clone(),
                        version,
                    },
                    value,
                ));
            }
            match status {
                SubstateStatus::Up => {
                    // No value, but the substate is up? That can't be right.
                    return Err(SubstateApiError::InvalidValidatorNodeResponse(
                        "Substate is up but no value was returned".to_string(),
                    ));
                },
                SubstateStatus::Down => {
                    version += 1;
                },
                SubstateStatus::DoesNotExist => {
                    return Err(SubstateApiError::SubstateDoesNotExist {
                        address: address.clone(),
                    })
                },
            }
        }
    }

    fn connect_validator_node(&self) -> Result<ValidatorNodeClient, SubstateApiError> {
        let client = ValidatorNodeClient::connect(self.validator_node_jrpc_endpoint)?;
        Ok(client)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SubstateApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
    #[error("Validator node client error: {0}")]
    ValidatorNodeClientError(#[from] ValidatorNodeClientError),
    #[error("Invalid validator node response: {0}")]
    InvalidValidatorNodeResponse(String),
    #[error("Substate {address} does not exist")]
    SubstateDoesNotExist { address: SubstateAddress },
}

impl IsNotFoundError for SubstateApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::SubstateDoesNotExist { .. }) ||
            matches!(self, Self::StoreError(e) if e.is_not_found_error()) ||
            matches!(self, Self::ValidatorNodeClientError(e) if e.is_not_found_error())
    }
}
