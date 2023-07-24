//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    ops::RangeInclusive,
    sync::Arc,
};

use async_trait::async_trait;
use tari_dan_common_types::{
    committee::{Committee, CommitteeShard},
    hashing::{ValidatorNodeBalancedMerkleTree, ValidatorNodeMerkleProof},
    Epoch,
    ShardId,
};
use tari_dan_storage::global::models::ValidatorNode;
use tari_epoch_manager::{EpochManagerError, EpochManagerReader};
use tokio::sync::{Mutex, MutexGuard};

use crate::support::{address::TestAddress, helpers::random_shard_in_bucket};

#[derive(Debug, Clone)]
pub struct TestEpochManager {
    inner: Arc<Mutex<TestEpochManagerState>>,
    our_validator_node: Option<ValidatorNode<TestAddress>>,
}

impl TestEpochManager {
    pub fn new() -> Self {
        Self {
            inner: Default::default(),
            our_validator_node: None,
        }
    }

    pub async fn state_lock(&self) -> MutexGuard<TestEpochManagerState> {
        self.inner.lock().await
    }

    pub fn clone_for(&self, address: TestAddress, shard_key: ShardId) -> Self {
        let mut copy = self.clone();
        copy.our_validator_node = Some(ValidatorNode {
            address,
            shard_key,
            epoch: Epoch(0),
            committee_bucket: None,
        });
        copy
    }

    pub async fn add_committees(&self, committees: HashMap<u32, Committee<TestAddress>>) {
        let mut state = self.state_lock().await;
        let num_committees = committees.len() as u32;
        for (bucket, committee) in committees {
            for address in &committee.members {
                state
                    .validator_shards
                    .insert(*address, (bucket, random_shard_in_bucket(bucket, num_committees)));
                state.address_bucket.insert(*address, bucket);
            }

            state.committees.insert(bucket, committee);
        }
    }

    pub async fn all_validators(&self) -> Vec<(TestAddress, u32, ShardId)> {
        self.state_lock()
            .await
            .validator_shards
            .iter()
            .map(|(a, (bucket, shard))| (*a, *bucket, *shard))
            .collect()
    }

    pub async fn all_committees(&self) -> HashMap<u32, Committee<TestAddress>> {
        self.state_lock().await.committees.clone()
    }
}

#[async_trait]
impl EpochManagerReader for TestEpochManager {
    type Addr = TestAddress;

    async fn get_committee(&self, _epoch: Epoch, shard: ShardId) -> Result<Committee<Self::Addr>, EpochManagerError> {
        let state = self.state_lock().await;
        let bucket = shard.to_committee_bucket(state.committees.len() as u32);
        Ok(state.committees[&bucket].clone())
    }

    async fn get_our_validator_node(&self, _epoch: Epoch) -> Result<ValidatorNode<TestAddress>, EpochManagerError> {
        Ok(self.our_validator_node.clone().unwrap())
    }

    async fn get_validator_node(
        &self,
        epoch: Epoch,
        addr: &Self::Addr,
    ) -> Result<ValidatorNode<Self::Addr>, EpochManagerError> {
        let (bucket, shard_key) = self.state_lock().await.validator_shards[addr];

        Ok(ValidatorNode {
            address: *addr,
            shard_key,
            epoch,
            committee_bucket: Some(bucket),
        })
    }

    async fn get_local_committee_shard(&self, epoch: Epoch) -> Result<CommitteeShard, EpochManagerError> {
        let our_vn = self.get_our_validator_node(epoch).await?;
        let num_committees = self.get_num_committees(epoch).await?;
        let committee = self.get_committee(epoch, our_vn.shard_key).await?;
        let our_bucket = our_vn.shard_key.to_committee_bucket(num_committees);

        Ok(CommitteeShard::new(num_committees, committee.len() as u32, our_bucket))
    }

    async fn current_epoch(&self) -> Result<Epoch, EpochManagerError> {
        Ok(self.inner.lock().await.current_epoch)
    }

    async fn is_epoch_active(&self, _epoch: Epoch) -> Result<bool, EpochManagerError> {
        Ok(self.inner.lock().await.is_epoch_active)
    }

    async fn get_num_committees(&self, _epoch: Epoch) -> Result<u32, EpochManagerError> {
        Ok(self.inner.lock().await.committees.len() as u32)
    }

    async fn get_validator_node_merkle_proof(
        &self,
        _epoch: Epoch,
    ) -> Result<ValidatorNodeMerkleProof, EpochManagerError> {
        let leaves = vec![];
        let tree = ValidatorNodeBalancedMerkleTree::create(leaves);
        Ok(ValidatorNodeMerkleProof::generate_proof(&tree, 0).unwrap())
    }

    async fn get_committees_by_buckets(
        &self,
        _epoch: Epoch,
        buckets: HashSet<u32>,
    ) -> Result<HashMap<u32, Committee<Self::Addr>>, EpochManagerError> {
        let state = self.state_lock().await;
        Ok(state
            .committees
            .iter()
            .filter(|(bucket, _)| buckets.contains(bucket))
            .map(|(bucket, committee)| (*bucket, committee.clone()))
            .collect())
    }

    async fn get_committee_shard(&self, epoch: Epoch, shard: ShardId) -> Result<CommitteeShard, EpochManagerError> {
        let num_committees = self.get_num_committees(epoch).await?;
        let committee = self.get_committee(epoch, shard).await?;
        let bucket = shard.to_committee_bucket(num_committees);

        Ok(CommitteeShard::new(num_committees, committee.len() as u32, bucket))
    }

    async fn get_committees_by_shards(
        &self,
        epoch: Epoch,
        shards: &HashSet<ShardId>,
    ) -> Result<HashMap<ShardId, Committee<Self::Addr>>, EpochManagerError> {
        let mut committees = HashMap::new();
        for shard in shards {
            committees.insert(*shard, self.get_committee(epoch, *shard).await?);
        }
        Ok(committees)
    }

    async fn get_committee_within_shard_range(
        &self,
        _epoch: Epoch,
        range: RangeInclusive<ShardId>,
    ) -> Result<Committee<Self::Addr>, EpochManagerError> {
        let lock = self.state_lock().await;
        Ok(Committee::new(
            lock.validator_shards
                .iter()
                .filter(|(_, (_, s))| range.contains(s))
                .map(|(a, _)| a)
                .copied()
                .collect(),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct TestEpochManagerState {
    pub current_epoch: Epoch,
    pub is_epoch_active: bool,
    pub validator_shards: HashMap<TestAddress, (u32, ShardId)>,
    pub committees: HashMap<u32, Committee<TestAddress>>,
    pub address_bucket: HashMap<TestAddress, u32>,
}

impl Default for TestEpochManagerState {
    fn default() -> Self {
        Self {
            current_epoch: Epoch(0),
            validator_shards: HashMap::new(),
            is_epoch_active: true,
            committees: HashMap::new(),
            address_bucket: HashMap::new(),
        }
    }
}
