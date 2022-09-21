use thiserror::Error;

use crate::{services::epoch_manager::EpochManagerError, storage::shard_store::StoreError, DigitalAssetError};

#[derive(Error, Debug)]
pub enum HotStuffError {
    #[error("Epoch manager error: {0}")]
    EpochManagerError(#[from] EpochManagerError),
    #[error("Received message from a node that is not in the committee")]
    ReceivedMessageFromNonCommitteeMember,
    #[error("Store error: {0}")]
    StoreError(#[from] StoreError),
    #[error("Claim is not valid")]
    ClaimIsNotValid,
    #[error("Node payload does not match justify payload")]
    NodePayloadDoesNotMatchJustifyPayload,
    #[error("Send error")]
    SendError,
    #[error("Not the leader")]
    NotTheLeader,

    #[error("DigitalAssetError: {0}")]
    DigitalAssetError(#[from] DigitalAssetError),
}
