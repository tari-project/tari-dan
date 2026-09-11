//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_ootle_common_types::{Epoch, SubstateAddress, optional::IsNotFoundError};
use tari_ootle_storage::StorageError;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

#[derive(thiserror::Error, Debug)]
pub enum EpochManagerError {
    #[error("Could not receive from channel")]
    ReceiveError,
    #[error("Could not send to channel")]
    SendError,
    #[error("No epoch found {0:?}")]
    NoEpochFound(Epoch),
    #[error("No committee found for shard {0:?}")]
    NoCommitteeFound(SubstateAddress),
    #[error("Unexpected request")]
    UnexpectedRequest,
    #[error("Unexpected response")]
    UnexpectedResponse,
    #[error("Storage error: {0}")]
    StorageError(StorageError),
    #[error("No validator nodes found for current shard key")]
    ValidatorNodesNotFound,
    #[error("No committee VNs found for address {substate_address} and epoch {epoch}")]
    NoCommitteeVns {
        substate_address: SubstateAddress,
        epoch: Epoch,
    },
    #[error("Validator node {address} is not registered at epoch {epoch}")]
    ValidatorNodeNotRegistered { address: String, epoch: Epoch },
    #[error("Invalid public key bytes: {public_key}")]
    InvalidPublicKeyBytes { public_key: RistrettoPublicKeyBytes },
    #[error("Integer overflow: {func}")]
    IntegerOverflow { func: &'static str },
    #[error("Invalid epoch: {epoch}")]
    InvalidEpoch { epoch: Epoch },
    #[error("Validator node registration sidechain id mismatch. Actual: {actual:?}, Expected: {expected:?}")]
    ValidatorNodeRegistrationSidechainIdMismatch {
        actual: Option<String>,
        expected: Option<String>,
    },
    #[error("Failed to submit layer one transaction: {details}")]
    FailedToSubmitLayerOneTransaction { details: String },
    #[error("Epoch event oracle error: {details}")]
    EpochEventOracleError { details: String },
}

impl EpochManagerError {
    pub fn is_not_registered_error(&self) -> bool {
        matches!(self, Self::ValidatorNodeNotRegistered { .. })
    }
}

impl IsNotFoundError for EpochManagerError {
    fn is_not_found_error(&self) -> bool {
        match self {
            Self::ValidatorNodeNotRegistered { .. } | Self::NoEpochFound(_) => true,
            Self::StorageError(err) => err.is_not_found_error(),
            _ => false,
        }
    }
}
