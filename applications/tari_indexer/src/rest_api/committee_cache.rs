//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::Arc;

use tari_indexer_client::types::ValidatorSetCommittee;
use tari_ootle_common_types::{Epoch, ValidatorSetHash, shard::Shard};

/// The committees of an epoch's shard groups, as proved against one validator set hash.
type EpochCommittees = Arc<Vec<ValidatorSetCommittee>>;

/// Committee proofs keyed by the epoch and the validator set hash they prove against. A validator set hash commits to
/// every committee of its epoch, so an entry never goes stale: a set the epoch manager reassigns has a different hash
/// and so a different key. Every wallet that walks the validator set asks for the same committee of each epoch, which
/// makes the proofs worth keeping.
#[derive(Clone)]
pub struct CommitteeProofCache {
    cache: mini_moka::sync::Cache<(Epoch, ValidatorSetHash), EpochCommittees>,
}

impl CommitteeProofCache {
    /// A cache that holds committees up to about `max_bytes` of members and proofs.
    pub fn new(max_bytes: u64) -> Self {
        let cache = mini_moka::sync::Cache::builder()
            .weigher(|_, committees: &EpochCommittees| {
                let bytes = committees.iter().map(committee_size).sum::<usize>();
                u32::try_from(bytes).unwrap_or(u32::MAX)
            })
            .max_capacity(max_bytes)
            .build();
        Self { cache }
    }

    /// `epoch`'s committee for the shard group that contains `shard`, proved against `validator_set_hash`.
    pub fn get(
        &self,
        epoch: Epoch,
        validator_set_hash: &ValidatorSetHash,
        shard: Shard,
    ) -> Option<ValidatorSetCommittee> {
        self.cache
            .get(&(epoch, *validator_set_hash))?
            .iter()
            .find(|committee| committee.shard_group.contains(&shard))
            .cloned()
    }

    /// Adds `committee` to the committees of its epoch and validator set hash.
    pub fn insert(&self, committee: ValidatorSetCommittee) {
        let key = (committee.epoch, committee.validator_set_hash);
        let mut committees = self
            .cache
            .get(&key)
            .map(|committees| committees.as_ref().clone())
            .unwrap_or_default();
        if committees
            .iter()
            .any(|cached| cached.shard_group == committee.shard_group)
        {
            return;
        }
        committees.push(committee);
        self.cache.insert(key, Arc::new(committees));
    }
}

fn committee_size(committee: &ValidatorSetCommittee) -> usize {
    size_of::<ValidatorSetCommittee>() +
        committee.proof.len() +
        committee.members.len() * size_of::<tari_indexer_client::types::ValidatorSetCommitteeMember>()
}

#[cfg(test)]
mod tests {
    use tari_indexer_client::types::ValidatorSetCommitteeMember;
    use tari_ootle_common_types::ShardGroup;

    use super::*;

    fn committee(epoch: u64, hash: u8, shard_group: ShardGroup) -> ValidatorSetCommittee {
        ValidatorSetCommittee {
            epoch: Epoch(epoch),
            shard_group,
            validator_set_hash: ValidatorSetHash::from_array([hash; 32]),
            members: vec![ValidatorSetCommitteeMember {
                public_key: Default::default(),
                vote_power: 1,
            }],
            proof: vec![hash; 64],
        }
    }

    #[test]
    fn a_committee_is_found_by_any_shard_of_its_group() {
        let cache = CommitteeProofCache::new(1 << 20);
        let hash = ValidatorSetHash::from_array([1; 32]);
        cache.insert(committee(5, 1, ShardGroup::new(0u32, 127u32)));
        cache.insert(committee(5, 1, ShardGroup::new(128u32, 255u32)));

        assert_eq!(
            cache.get(Epoch(5), &hash, Shard::from(0u32)).unwrap().shard_group,
            ShardGroup::new(0u32, 127u32)
        );
        assert_eq!(
            cache.get(Epoch(5), &hash, Shard::from(200u32)).unwrap().shard_group,
            ShardGroup::new(128u32, 255u32)
        );
    }

    #[test]
    fn a_committee_is_found_only_under_the_hash_it_proves_against() {
        let cache = CommitteeProofCache::new(1 << 20);
        cache.insert(committee(5, 1, ShardGroup::new(0u32, 255u32)));

        assert!(
            cache
                .get(Epoch(5), &ValidatorSetHash::from_array([2; 32]), Shard::from(0u32))
                .is_none()
        );
        assert!(
            cache
                .get(Epoch(6), &ValidatorSetHash::from_array([1; 32]), Shard::from(0u32))
                .is_none()
        );
    }
}
