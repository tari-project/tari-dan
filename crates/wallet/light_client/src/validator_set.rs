//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ootle_network::Network;
use serde::{Deserialize, Serialize};
use tari_engine_types::limits::MAX_CBOR_NESTING_DEPTH;
use tari_indexer_client::types::{EpochCheckpointEntry, ValidatorSetCommittee};
use tari_ootle_common_types::{Epoch, NumPreshards, ValidatorSetHash, shard::Shard};
use tari_ootle_storage::consensus_models::EpochCheckpoint;

use crate::{LightClientError, VerifiedCommittee};

/// The latest validator set the light client trusts, with the committee of the shard group it walks.
///
/// The light client follows one shard group, the one that holds `walk_shard` in each epoch, because shard group
/// boundaries move when the number of committees changes. Each end-of-epoch checkpoint of that shard group, signed by
/// the committee held here, names the next epoch's validator set, and [`Self::advance`] moves the trust to it.
///
/// Trust starts at an anchor: the first validator set the light client accepted without a signed handover, either on
/// first use ([`Self::anchor_on_first_use`]) or from a pinned hash ([`Self::anchor_pinned`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedValidatorSet {
    walk_shard: Shard,
    anchor_epoch: Epoch,
    committee: VerifiedCommittee,
}

impl TrustedValidatorSet {
    /// Trusts the validator set that the checkpoint in `entry` names for the next epoch.
    ///
    /// The checkpoint's quorum certificate cannot be checked, since no committee is trusted yet, so this takes the
    /// indexer at its word on which validator set is the network's. Every later read verifies against that set, so a
    /// forged anchor holds only while the indexer that served it keeps serving the light client.
    pub fn anchor_on_first_use(
        network: Network,
        walk_shard: Shard,
        entry: &EpochCheckpointEntry,
    ) -> Result<Self, LightClientError> {
        let checkpoint = decode_checkpoint(network, entry)?;
        let epoch = checkpoint.epoch();
        checkpoint
            .validate_well_formed()
            .map_err(|source| LightClientError::InvalidCheckpoint { epoch, source })?;
        let next_set = next_validator_set(&checkpoint)?;
        let committee = next_committee(entry, epoch)?;
        Self::anchor_pinned(network, walk_shard, epoch + Epoch(1), &next_set, committee)
    }

    /// Trusts `validator_set_hash` as `epoch`'s validator set, with `committee` proved as the walked shard group's
    /// committee in it.
    pub fn anchor_pinned(
        network: Network,
        walk_shard: Shard,
        epoch: Epoch,
        validator_set_hash: &ValidatorSetHash,
        committee: &ValidatorSetCommittee,
    ) -> Result<Self, LightClientError> {
        let committee = verify_walked_committee(network, walk_shard, epoch, validator_set_hash, committee)?;
        Ok(Self {
            walk_shard,
            anchor_epoch: epoch,
            committee,
        })
    }

    pub fn network(&self) -> Network {
        self.committee.network()
    }

    /// The epoch whose validator set this is.
    pub fn epoch(&self) -> Epoch {
        self.committee.epoch()
    }

    pub fn validator_set_hash(&self) -> &ValidatorSetHash {
        self.committee.validator_set_hash()
    }

    pub fn num_preshards(&self) -> NumPreshards {
        self.committee.num_preshards()
    }

    pub fn walk_shard(&self) -> Shard {
        self.walk_shard
    }

    /// The epoch of the validator set the light client last anchored on. Every set since was handed over by a
    /// committee of the one before.
    pub fn anchor_epoch(&self) -> Epoch {
        self.anchor_epoch
    }

    /// The committee of the shard group that holds [`Self::walk_shard`] in [`Self::epoch`].
    pub fn walk_committee(&self) -> &VerifiedCommittee {
        &self.committee
    }

    /// Verifies `committee` as one of this validator set's committees, of any shard group.
    pub fn verify_committee(&self, committee: &ValidatorSetCommittee) -> Result<VerifiedCommittee, LightClientError> {
        VerifiedCommittee::verify(self.network(), self.epoch(), self.validator_set_hash(), committee)
    }

    /// Moves the trust to the next epoch's validator set through the end-of-epoch checkpoint in `entry`, which the
    /// walked shard group's committee must have signed.
    ///
    /// Walking from epoch N to M is one `advance` per checkpoint of N to M - 1, in order. A checkpoint that fails
    /// leaves `self` as it was, and the walk stops there: a failed walk proves the indexer wrong, never the trusted
    /// set.
    pub fn advance(&self, entry: &EpochCheckpointEntry) -> Result<Self, LightClientError> {
        let network = self.network();
        let epoch = self.epoch();
        let checkpoint = decode_checkpoint(network, entry)?;
        if checkpoint.epoch() != epoch {
            return Err(LightClientError::CheckpointEpochMismatch {
                expected: epoch,
                found: checkpoint.epoch(),
            });
        }
        let shard_group = checkpoint
            .checked_shard_group()
            .map_err(|source| LightClientError::InvalidCheckpoint { epoch, source })?;
        if shard_group != self.committee.shard_group() {
            return Err(LightClientError::CheckpointShardGroupMismatch {
                epoch,
                expected: self.committee.shard_group(),
                found: shard_group,
            });
        }
        checkpoint
            .validate(epoch, self.committee.quorum_threshold(), |pk| {
                Ok(self.committee.vote_power(pk))
            })
            .map_err(|source| LightClientError::InvalidCheckpoint { epoch, source })?;

        let next_set = next_validator_set(&checkpoint)?;
        let committee = next_committee(entry, epoch)?;
        let committee = verify_walked_committee(network, self.walk_shard, epoch + Epoch(1), &next_set, committee)?;
        Ok(Self {
            walk_shard: self.walk_shard,
            anchor_epoch: self.anchor_epoch,
            committee,
        })
    }
}

fn decode_checkpoint(network: Network, entry: &EpochCheckpointEntry) -> Result<EpochCheckpoint, LightClientError> {
    let checkpoint: EpochCheckpoint = tari_bor::decode_with_max_depth(&entry.checkpoint, MAX_CBOR_NESTING_DEPTH)
        .map_err(|e| LightClientError::Decode {
            what: "epoch checkpoint",
            details: e.to_string(),
        })?;
    let found = checkpoint.header().network;
    if found != network.as_byte() {
        return Err(LightClientError::WrongNetwork {
            what: "epoch checkpoint",
            expected: network,
            found,
        });
    }
    Ok(checkpoint)
}

/// The validator set the checkpoint's end-of-epoch block names for the next epoch. Committed once the checkpoint
/// validates, since validation checks the preimage against the header's metadata hash.
fn next_validator_set(checkpoint: &EpochCheckpoint) -> Result<ValidatorSetHash, LightClientError> {
    checkpoint
        .next_epoch_validator_set_hash()
        .ok_or(LightClientError::NoNextValidatorSet {
            epoch: checkpoint.epoch(),
        })
}

fn next_committee(entry: &EpochCheckpointEntry, epoch: Epoch) -> Result<&ValidatorSetCommittee, LightClientError> {
    entry
        .next_epoch_committee
        .as_ref()
        .ok_or(LightClientError::MissingNextCommittee { epoch })
}

fn verify_walked_committee(
    network: Network,
    walk_shard: Shard,
    epoch: Epoch,
    validator_set_hash: &ValidatorSetHash,
    committee: &ValidatorSetCommittee,
) -> Result<VerifiedCommittee, LightClientError> {
    let committee = VerifiedCommittee::verify(network, epoch, validator_set_hash, committee)?;
    if !committee.shard_group().contains(&walk_shard) {
        return Err(LightClientError::CommitteeOutsideWalkedShard {
            shard_group: committee.shard_group(),
            shard: walk_shard,
        });
    }
    Ok(committee)
}
