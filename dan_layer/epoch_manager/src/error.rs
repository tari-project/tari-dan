//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::PublicKey;
use tari_dan_common_types::{optional::IsNotFoundError, Epoch, SubstateAddress};

#[derive(thiserror::Error, Debug)]
pub enum EpochManagerError {
    #[error("Could not receive from channel")]
    ReceiveError,
    #[error("Could not send to channel")]
    SendError,
    #[error("Base node errored: {0}")]
    BaseNodeError(anyhow::Error),
    #[error("No epoch found {0:?}")]
    NoEpochFound(Epoch),
    #[error("No committee found for shard {0:?}")]
    NoCommitteeFound(SubstateAddress),
    #[error("Unexpected request")]
    UnexpectedRequest,
    #[error("Unexpected response")]
    UnexpectedResponse,
    #[error("SQLite Storage error: {0}")]
    SqlLiteStorageError(anyhow::Error),
    #[error("No validator nodes found for current shard key")]
    ValidatorNodesNotFound,
    #[error("No committee VNs found for shard {substate_address} and epoch {epoch}")]
    NoCommitteeVns {
        substate_address: SubstateAddress,
        epoch: Epoch,
    },
    #[error("Validator node {address} is not registered at epoch {epoch}")]
    ValidatorNodeNotRegistered { address: String, epoch: Epoch },
    #[error("Base layer consensus constants not set")]
    BaseLayerConsensusConstantsNotSet,
    #[error("Base layer could not return shard key for {public_key} at height {block_height}")]
    ShardKeyNotFound { public_key: PublicKey, block_height: u64 },
    #[error("BalancedBinaryMerkleTreeError: {0}")]
    BalancedBinaryMerkleTreeError(anyhow::Error),
    #[error("BalancedBinaryMerkleProofError: {0}")]
    BalancedBinaryMerkleProofError(anyhow::Error),
    #[error("Integer overflow: {func}")]
    IntegerOverflow { func: &'static str },
    #[error("Invalid epoch: {epoch}")]
    InvalidEpoch { epoch: Epoch },
    #[error("Could not create SMT: {0}")]
    SmtError(#[from] tari_mmr::sparse_merkle_tree::SMTError)
}

impl EpochManagerError {
    pub fn is_not_registered_error(&self) -> bool {
        matches!(self, Self::ValidatorNodeNotRegistered { .. })
    }
}

impl IsNotFoundError for EpochManagerError {
    fn is_not_found_error(&self) -> bool {
        self.is_not_registered_error()
    }
}
