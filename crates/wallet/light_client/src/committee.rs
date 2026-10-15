//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ootle_network::Network;
use serde::{Deserialize, Serialize};
use tari_indexer_client::types::ValidatorSetCommittee;
use tari_ootle_common_types::{Epoch, NumPreshards, ShardGroup, ValidatorSetHash, ValidatorSetMember, VotePower};
use tari_ootle_storage::consensus_models::{CommittedBlockProof, VerifiedBlockTip};
use tari_state_tree::ValidatorSetCommitteeProof;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use crate::LightClientError;

/// One shard group's committee in an epoch, proved to be that shard group's committee in a validator set the light
/// client trusts.
///
/// The light client builds a `VerifiedCommittee` only by verifying a committee proof against a trusted validator set
/// hash, so a commit proof that this committee signs is one the network committed. Deserializing one trusts the store
/// it is read from as much as the light client that wrote it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedCommittee {
    network: Network,
    epoch: Epoch,
    num_preshards: NumPreshards,
    shard_group: ShardGroup,
    validator_set_hash: ValidatorSetHash,
    /// Ascending by public key.
    members: Vec<ValidatorSetMember>,
}

impl VerifiedCommittee {
    /// Verifies `committee` as a committee of the validator set `validator_set_hash`, which the caller trusts to be
    /// `epoch`'s validator set on `network`. Only the proof is read: every other field of `committee` is untrusted.
    pub(crate) fn verify(
        network: Network,
        epoch: Epoch,
        validator_set_hash: &ValidatorSetHash,
        committee: &ValidatorSetCommittee,
    ) -> Result<Self, LightClientError> {
        let proof = ValidatorSetCommitteeProof::from_bytes(&committee.proof)?;
        // The set hash commits to the network and epoch, so a proof for another one fails `verify`. These checks name
        // the mismatch instead.
        if proof.network != network {
            return Err(LightClientError::WrongNetwork {
                what: "committee proof",
                expected: network,
                found: proof.network.as_byte(),
            });
        }
        if proof.epoch != epoch {
            return Err(LightClientError::CommitteeEpochMismatch {
                expected: epoch,
                found: proof.epoch,
            });
        }
        let members = proof.verify(validator_set_hash)?.to_vec();
        // A committee with no vote power has a quorum threshold of zero, which a certificate without signatures meets.
        if members.iter().all(|member| member.vote_power.is_zero()) {
            return Err(LightClientError::EmptyCommittee {
                epoch,
                shard_group: proof.shard_group,
            });
        }
        Ok(Self {
            network,
            epoch,
            num_preshards: proof.num_preshards,
            shard_group: proof.shard_group,
            validator_set_hash: *validator_set_hash,
            members,
        })
    }

    pub fn network(&self) -> Network {
        self.network
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn num_preshards(&self) -> NumPreshards {
        self.num_preshards
    }

    pub fn shard_group(&self) -> ShardGroup {
        self.shard_group
    }

    /// The validator set this committee was proved against.
    pub fn validator_set_hash(&self) -> &ValidatorSetHash {
        &self.validator_set_hash
    }

    /// Ascending by public key.
    pub fn members(&self) -> &[ValidatorSetMember] {
        &self.members
    }

    pub fn total_power(&self) -> VotePower {
        self.members
            .iter()
            .fold(VotePower::zero(), |acc, member| acc.saturating_add(member.vote_power))
    }

    /// The quorum threshold $n - f$, where $n$ is the committee's total vote power and $f = (n - 1) / 3$ the faulty
    /// power it tolerates.
    pub fn quorum_threshold(&self) -> VotePower {
        let total = self.total_power();
        total - (total - VotePower::of(1)) / VotePower::of(3)
    }

    /// The vote power of `public_key` in this committee, zero for a non-member.
    pub fn vote_power(&self, public_key: &RistrettoPublicKeyBytes) -> VotePower {
        self.members
            .binary_search_by(|member| member.public_key.cmp(public_key))
            .map(|i| self.members[i].vote_power)
            .unwrap_or_else(|_| VotePower::zero())
    }

    /// Verifies that this committee committed the block `commit_proof` proves, returning the block's quorum-signed
    /// header fields.
    pub fn verify_commit_proof(&self, commit_proof: &[u8]) -> Result<VerifiedBlockTip, LightClientError> {
        let commit_proof = CommittedBlockProof::from_bytes(commit_proof)?;
        let network = commit_proof.proof().header.network;
        if network != self.network.as_byte() {
            return Err(LightClientError::WrongNetwork {
                what: "commit proof",
                expected: self.network,
                found: network,
            });
        }
        if commit_proof.epoch() != self.epoch {
            return Err(LightClientError::CommitProofEpochMismatch {
                expected: self.epoch,
                found: commit_proof.epoch(),
            });
        }
        let shard_group = commit_proof.shard_group()?;
        if shard_group != self.shard_group {
            return Err(LightClientError::CommitProofShardGroupMismatch {
                expected: self.shard_group,
                found: shard_group,
            });
        }
        let tip = commit_proof.validate(self.quorum_threshold(), |pk| Ok(self.vote_power(pk)))?;
        Ok(tip)
    }
}
