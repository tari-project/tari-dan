//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use tari_ootle_common_types::{Epoch, SubstateAddress, SubstateVersion, displayable::Displayable};
use tari_sidechain::FixedHash;
use tari_template_lib_types::{Hash32, crypto::RistrettoPublicKeyBytes};

#[derive(Debug)]
pub enum EpochEvent {
    Error(anyhow::Error),
    ActiveValidatorNodeSetChanged {
        epoch: Epoch,
        node_changes: Vec<ValidatorNodeChange>,
    },
    NewValidatorRegistered {
        epoch: Epoch,
        claim_public_key: RistrettoPublicKeyBytes,
        validator_node_public_key: RistrettoPublicKeyBytes,
    },
    NewValidatorNodeExit {
        epoch: Epoch,
        validator_node_public_key: RistrettoPublicKeyBytes,
    },
    EpochChanged {
        epoch: Epoch,
        epoch_hash: FixedHash,
    },
    DoneForNow {
        epoch: Epoch,
        epoch_hash: FixedHash,
    },
}

impl EpochEvent {
    pub fn error<E: Into<anyhow::Error>>(e: E) -> Self {
        EpochEvent::Error(e.into())
    }
}

impl Display for EpochEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EpochEvent::Error(e) => write!(f, "Error: {}", e),
            EpochEvent::ActiveValidatorNodeSetChanged { epoch, node_changes } => {
                write!(
                    f,
                    "ActiveValidatorNodeSetChanged {{ epoch: {}, node_changes: {} }}",
                    epoch,
                    node_changes.display()
                )
            },
            EpochEvent::NewValidatorRegistered {
                epoch,
                claim_public_key,
                validator_node_public_key,
            } => {
                write!(
                    f,
                    "NewValidatorRegistered {{ epoch: {}, claim_public_key: {}, validator_node_public_key: {} }}",
                    epoch, claim_public_key, validator_node_public_key
                )
            },
            EpochEvent::NewValidatorNodeExit {
                epoch,
                validator_node_public_key,
            } => {
                write!(
                    f,
                    "NewValidatorNodeExit {{ epoch: {}, validator_node_public_key: {} }}",
                    epoch, validator_node_public_key
                )
            },
            EpochEvent::EpochChanged { epoch, epoch_hash } => {
                write!(f, "EpochChanged {{ epoch: {}, epoch_hash: {} }}", epoch, epoch_hash)
            },
            EpochEvent::DoneForNow { epoch, epoch_hash } => {
                write!(f, "DoneForNow {{ epoch: {}, hash: {} }}", epoch, epoch_hash)
            },
        }
    }
}

/// Represents a validator node state change
#[derive(Debug, Clone)]
pub enum ValidatorNodeChange {
    Add {
        claim_public_key: RistrettoPublicKeyBytes,
        validator_node_public_key: RistrettoPublicKeyBytes,
        activation_epoch: Epoch,
        minimum_value_promise: u64,
        // TODO: replace with ValidatorShardKey type
        shard_key: SubstateAddress,
    },
    Remove {
        public_key: RistrettoPublicKeyBytes,
    },
}

impl TryFrom<minotari_app_grpc::tari_rpc::ValidatorNodeChange> for ValidatorNodeChange {
    type Error = anyhow::Error;

    fn try_from(value: minotari_app_grpc::tari_rpc::ValidatorNodeChange) -> Result<Self, Self::Error> {
        use anyhow::Context;
        match value.change {
            Some(minotari_app_grpc::tari_rpc::validator_node_change::Change::Add(add)) => {
                let registration = add
                    .registration
                    .ok_or_else(|| anyhow::anyhow!("ValidatorNodeChange Add missing registration field"))?;
                let claim_public_key = RistrettoPublicKeyBytes::from_bytes(&registration.claim_public_key)
                    .context("Invalid claim_public_key")?;
                let validator_node_public_key = RistrettoPublicKeyBytes::from_bytes(&registration.public_key)
                    .context("Invalid validator_node_public_key")?;
                Ok(ValidatorNodeChange::Add {
                    claim_public_key,
                    validator_node_public_key,
                    activation_epoch: Epoch(add.activation_epoch),
                    minimum_value_promise: add.minimum_value_promise,
                    shard_key: {
                        let hash = Hash32::try_from(add.shard_key.as_slice()).context("Invalid shard key hash")?;
                        SubstateAddress::from_hash_and_version(hash, SubstateVersion::ZERO)
                    },
                })
            },
            Some(minotari_app_grpc::tari_rpc::validator_node_change::Change::Remove(remove)) => {
                Ok(ValidatorNodeChange::Remove {
                    public_key: RistrettoPublicKeyBytes::from_bytes(&remove.public_key)
                        .context("invalid public key in ValidatorNodeChange::Remove")?,
                })
            },
            None => Err(anyhow::anyhow!("ValidatorNodeChange missing change field")),
        }
    }
}

impl Display for ValidatorNodeChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidatorNodeChange::Add {
                claim_public_key,
                validator_node_public_key,
                activation_epoch,
                minimum_value_promise,
                shard_key,
            } => write!(
                f,
                "Add {{ claim_public_key: {}, validator_node_public_key: {}, activation_epoch: {}, \
                 minimum_value_promise: {}, shard_key: {} }}",
                claim_public_key, validator_node_public_key, activation_epoch, minimum_value_promise, shard_key
            ),
            ValidatorNodeChange::Remove { public_key } => write!(f, "Remove {{ public_key: {} }}", public_key),
        }
    }
}
