//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use anyhow::anyhow;
use async_trait::async_trait;
use tari_common_types::types::FixedHash;
use tari_consensus::traits::{EpochManager, EpochManagerError};
use tari_dan_common_types::{
    committee::{Committee, CommitteeShard},
    hasher::tari_hasher,
    hashing::{TariDanConsensusHashDomain, ValidatorNodeBalancedMerkleTree, ValidatorNodeMerkleProof},
    Epoch,
    ShardId,
};
use tokio::sync::{Mutex, MutexGuard};

use crate::support::{address::TestAddress, helpers::random_shard_in_bucket};

#[derive(Debug, Clone)]
pub struct TestEpochManager {
    inner: Arc<Mutex<TestEpochManagerState>>,
    our_validator_shard: Option<ShardId>,
    our_validator_addr: Option<TestAddress>,
}

impl TestEpochManager {
    pub fn new() -> Self {
        Self {
            inner: Default::default(),
            our_validator_shard: None,
            our_validator_addr: None,
        }
    }

    pub async fn state_lock(&self) -> MutexGuard<TestEpochManagerState> {
        self.inner.lock().await
    }

    pub fn clone_for(&self, address: TestAddress, shard_id: ShardId) -> Self {
        let mut copy = self.clone();
        copy.our_validator_addr = Some(address);
        copy.our_validator_shard = Some(shard_id);
        copy
    }

    pub async fn add_committee(&self, bucket: u32, committee: Committee<TestAddress>) {
        let mut state = self.state_lock().await;
        let num_committees = state.num_committees;
        for address in &committee.members {
            state
                .validator_shards
                .insert(*address, random_shard_in_bucket(bucket, num_committees));
            state.address_bucket.insert(*address, bucket);
        }
        state.committees.insert(bucket, committee);
    }

    pub async fn set_num_committees(&self, num_committees: u32) {
        let mut state = self.state_lock().await;
        state.num_committees = num_committees;
    }

    pub async fn all_validators(&self) -> Vec<(TestAddress, ShardId)> {
        self.state_lock()
            .await
            .validator_shards
            .iter()
            .map(|(a, b)| (*a, *b))
            .collect()
    }

    pub async fn all_committees(&self) -> HashMap<u32, Committee<TestAddress>> {
        self.state_lock().await.committees.clone()
    }
}

#[async_trait]
impl EpochManager for TestEpochManager {
    type Addr = TestAddress;
    type Error = TestEpochManagerError;

    async fn get_committee(&self, _epoch: Epoch, shard: ShardId) -> Result<Committee<Self::Addr>, Self::Error> {
        let state = self.state_lock().await;
        let bucket = shard.to_committee_bucket(state.num_committees);
        Ok(state.committees[&bucket].clone())
    }

    async fn get_our_validator_shard(&self, _epoch: Epoch) -> Result<ShardId, Self::Error> {
        Ok(self.our_validator_shard.unwrap())
    }

    async fn get_validator_shard(&self, _epoch: Epoch, addr: &Self::Addr) -> Result<ShardId, Self::Error> {
        Ok(self.state_lock().await.validator_shards[addr])
    }

    async fn get_our_validator_addr(&self, _epoch: Epoch) -> Result<Self::Addr, Self::Error> {
        let state = self.state_lock().await;
        let addr = state
            .validator_shards
            .iter()
            .find(|(_, s)| **s == self.our_validator_shard.unwrap())
            .map(|(a, _)| *a)
            .ok_or(anyhow!("Our validator shard found for our address"))?;
        Ok(addr)
    }

    async fn get_local_committee_shard(&self, epoch: Epoch) -> Result<CommitteeShard, Self::Error> {
        let our_shard = self.get_our_validator_shard(epoch).await?;
        let num_committees = self.get_num_committees(epoch).await?;
        let committee = self.get_committee(epoch, our_shard).await?;
        let our_bucket = our_shard.to_committee_bucket(num_committees);

        Ok(CommitteeShard::new(num_committees, committee.len() as u32, our_bucket))
    }

    async fn current_epoch(&self) -> Result<Epoch, Self::Error> {
        Ok(self.inner.lock().await.current_epoch)
    }

    async fn is_epoch_active(&self, _epoch: Epoch) -> Result<bool, Self::Error> {
        Ok(self.inner.lock().await.is_epoch_active)
    }

    async fn get_num_committees(&self, _epoch: Epoch) -> Result<u32, Self::Error> {
        Ok(self.inner.lock().await.num_committees)
    }

    async fn get_validator_node_merkle_proof(&self, _epoch: Epoch) -> Result<ValidatorNodeMerkleProof, Self::Error> {
        let leaves = vec![];
        let tree = ValidatorNodeBalancedMerkleTree::create(leaves);
        Ok(ValidatorNodeMerkleProof::generate_proof(&tree, 0).unwrap())
    }

    async fn get_committees_by_buckets(
        &self,
        _epoch: Epoch,
        buckets: HashSet<u32>,
    ) -> Result<HashMap<u32, Committee<Self::Addr>>, Self::Error> {
        let state = self.state_lock().await;
        Ok(state
            .committees
            .iter()
            .filter(|(bucket, _)| buckets.contains(bucket))
            .map(|(bucket, committee)| (*bucket, committee.clone()))
            .collect())
    }

    async fn get_validator_leaf_hash(&self, epoch: Epoch, addr: &Self::Addr) -> Result<FixedHash, Self::Error> {
        let state = self.state_lock().await;
        let shard = state
            .validator_shards
            .get(addr)
            .ok_or_else(|| anyhow!("Validator {} not found in validator_shards for epoch {}", addr, epoch))?;

        Ok(tari_hasher::<TariDanConsensusHashDomain>("test-leaf-hash")
            .chain(&addr)
            .chain(shard)
            .result())
    }

    async fn get_committee_shard(&self, epoch: Epoch, shard: ShardId) -> Result<CommitteeShard, Self::Error> {
        let num_committees = self.get_num_committees(epoch).await?;
        let committee = self.get_committee(epoch, shard).await?;
        let bucket = shard.to_committee_bucket(num_committees);

        Ok(CommitteeShard::new(num_committees, committee.len() as u32, bucket))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("TestEpochManagerError: {0}")]
pub struct TestEpochManagerError(anyhow::Error);

impl From<anyhow::Error> for TestEpochManagerError {
    fn from(err: anyhow::Error) -> Self {
        Self(err)
    }
}

impl EpochManagerError for TestEpochManagerError {
    fn to_anyhow(&self) -> anyhow::Error {
        anyhow!("{}", self.0)
    }
}

#[derive(Debug, Clone)]
pub struct TestEpochManagerState {
    pub current_epoch: Epoch,
    pub is_epoch_active: bool,
    pub num_committees: u32,
    pub validator_shards: HashMap<TestAddress, ShardId>,
    pub committees: HashMap<u32, Committee<TestAddress>>,
    pub address_bucket: HashMap<TestAddress, u32>,
}

impl Default for TestEpochManagerState {
    fn default() -> Self {
        Self {
            current_epoch: Epoch(0),
            validator_shards: HashMap::new(),
            is_epoch_active: true,
            num_committees: 1,
            committees: HashMap::new(),
            address_bucket: HashMap::new(),
        }
    }
}
