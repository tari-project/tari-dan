//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    ops::RangeInclusive,
    sync::Arc,
};

use async_trait::async_trait;
use tari_common_types::types::{FixedHash, PublicKey};
use tari_dan_common_types::{
    committee::{Committee, CommitteeShard, CommitteeShardInfo},
    hashing::{MergedValidatorNodeMerkleProof, ValidatorNodeBalancedMerkleTree, ValidatorNodeMerkleProof},
    shard::Shard,
    Epoch,
    SubstateAddress,
};
use tari_dan_storage::global::models::ValidatorNode;
use tari_epoch_manager::{EpochManagerError, EpochManagerEvent, EpochManagerReader};
use tokio::sync::{broadcast, Mutex, MutexGuard};

use crate::support::{address::TestAddress, helpers::random_substate_in_bucket};

#[derive(Debug, Clone)]
pub struct TestEpochManager {
    inner: Arc<Mutex<TestEpochManagerState>>,
    our_validator_node: Option<ValidatorNode<TestAddress>>,
    tx_epoch_events: broadcast::Sender<EpochManagerEvent>,
}

impl TestEpochManager {
    pub fn new(tx_epoch_events: broadcast::Sender<EpochManagerEvent>) -> Self {
        Self {
            inner: Default::default(),
            our_validator_node: None,
            tx_epoch_events,
        }
    }

    pub async fn set_current_epoch(&self, current_epoch: Epoch) -> &Self {
        {
            let mut lock = self.inner.lock().await;
            lock.current_epoch = current_epoch;
            lock.is_epoch_active = true;
        }

        let _ = self
            .tx_epoch_events
            .send(EpochManagerEvent::EpochChanged(current_epoch));

        self
    }

    pub async fn state_lock(&self) -> MutexGuard<TestEpochManagerState> {
        self.inner.lock().await
    }

    pub fn clone_for(&self, address: TestAddress, public_key: PublicKey, shard_key: SubstateAddress) -> Self {
        let mut copy = self.clone();
        copy.our_validator_node = Some(ValidatorNode {
            address,
            public_key,
            shard_key,
            epoch: Epoch(0),
            committee_shard: None,
            fee_claim_public_key: PublicKey::default(),
        });
        copy
    }

    pub async fn add_committees(&self, committees: HashMap<Shard, Committee<TestAddress>>) {
        let mut state = self.state_lock().await;
        let num_committees = committees.len() as u32;
        for (shard, committee) in committees {
            for (address, pk) in &committee.members {
                let substate_address = random_substate_in_bucket(shard, num_committees);
                state.validator_shards.insert(
                    address.clone(),
                    (shard, substate_address.to_substate_address(), pk.clone()),
                );
                state.address_shard.insert(address.clone(), shard);
            }

            state.committees.insert(shard, committee);
        }
    }

    pub async fn all_validators(&self) -> Vec<(TestAddress, Shard, SubstateAddress, PublicKey)> {
        self.state_lock()
            .await
            .validator_shards
            .iter()
            .map(|(a, (shard, substate_address, pk))| (a.clone(), *shard, *substate_address, pk.clone()))
            .collect()
    }

    pub async fn all_committees(&self) -> HashMap<Shard, Committee<TestAddress>> {
        self.state_lock().await.committees.clone()
    }
}

#[async_trait]
impl EpochManagerReader for TestEpochManager {
    type Addr = TestAddress;

    async fn subscribe(&self) -> Result<broadcast::Receiver<EpochManagerEvent>, EpochManagerError> {
        Ok(self.tx_epoch_events.subscribe())
    }

    async fn get_committee(
        &self,
        _epoch: Epoch,
        substate_address: SubstateAddress,
    ) -> Result<Committee<Self::Addr>, EpochManagerError> {
        let state = self.state_lock().await;
        let shard = substate_address.to_committee_shard(state.committees.len() as u32);
        Ok(state.committees[&shard].clone())
    }

    async fn get_our_validator_node(&self, _epoch: Epoch) -> Result<ValidatorNode<TestAddress>, EpochManagerError> {
        Ok(self.our_validator_node.clone().unwrap())
    }

    async fn get_validator_node(
        &self,
        epoch: Epoch,
        addr: &Self::Addr,
    ) -> Result<ValidatorNode<Self::Addr>, EpochManagerError> {
        let (shard, shard_key, public_key) = self.state_lock().await.validator_shards[addr].clone();

        Ok(ValidatorNode {
            address: addr.clone(),
            public_key,
            shard_key,
            epoch,
            committee_shard: Some(shard),
            fee_claim_public_key: PublicKey::default(),
        })
    }

    async fn get_local_committee_shard(&self, epoch: Epoch) -> Result<CommitteeShard, EpochManagerError> {
        let our_vn = self.get_our_validator_node(epoch).await?;
        let num_committees = self.get_num_committees(epoch).await?;
        let committee = self.get_committee(epoch, our_vn.shard_key).await?;
        let our_shard = our_vn.shard_key.to_committee_shard(num_committees);

        Ok(CommitteeShard::new(num_committees, committee.len() as u32, our_shard))
    }

    async fn current_epoch(&self) -> Result<Epoch, EpochManagerError> {
        Ok(self.inner.lock().await.current_epoch)
    }

    async fn current_base_layer_block_info(&self) -> Result<(u64, FixedHash), EpochManagerError> {
        Ok(self.inner.lock().await.current_block_info)
    }

    async fn is_epoch_active(&self, _epoch: Epoch) -> Result<bool, EpochManagerError> {
        Ok(self.inner.lock().await.is_epoch_active)
    }

    async fn get_num_committees(&self, _epoch: Epoch) -> Result<u32, EpochManagerError> {
        Ok(self.inner.lock().await.committees.len() as u32)
    }

    async fn get_validator_node_merkle_root(&self, _epoch: Epoch) -> Result<Vec<u8>, EpochManagerError> {
        let leaves = vec![];
        let tree = ValidatorNodeBalancedMerkleTree::create(leaves);
        Ok(tree.get_merkle_root())
    }

    async fn get_validator_set_merged_merkle_proof(
        &self,
        _epoch: Epoch,
        _validators: Vec<PublicKey>,
    ) -> Result<MergedValidatorNodeMerkleProof, EpochManagerError> {
        let leaves = vec![];
        let tree = ValidatorNodeBalancedMerkleTree::create(leaves);
        let proof = ValidatorNodeMerkleProof::generate_proof(&tree, 0).unwrap();
        Ok(MergedValidatorNodeMerkleProof::create_from_proofs(&[proof]).unwrap())
    }

    async fn get_committees_by_shards(
        &self,
        _epoch: Epoch,
        shards: HashSet<Shard>,
    ) -> Result<HashMap<Shard, Committee<Self::Addr>>, EpochManagerError> {
        let state = self.state_lock().await;
        Ok(state
            .committees
            .iter()
            .filter(|(shard, _)| shards.contains(shard))
            .map(|(shard, committee)| (*shard, committee.clone()))
            .collect())
    }

    async fn get_committee_shard(
        &self,
        epoch: Epoch,
        substate_address: SubstateAddress,
    ) -> Result<CommitteeShard, EpochManagerError> {
        let num_committees = self.get_num_committees(epoch).await?;
        let committee = self.get_committee(epoch, substate_address).await?;
        let shard = substate_address.to_committee_shard(num_committees);

        Ok(CommitteeShard::new(num_committees, committee.len() as u32, shard))
    }

    // async fn get_committees_by_shards(
    //     &self,
    //     epoch: Epoch,
    //     shards: &HashSet<SubstateAddress>,
    // ) -> Result<HashMap<Shard, Committee<Self::Addr>>, EpochManagerError> { let num_committees =
    //   self.get_num_committees(epoch).await?;
    //
    //     let mut committees = HashMap::new();
    //     let buckets = shards.iter().map(|shard| shard.to_committee_bucket(num_committees));
    //     let state = self.state_lock().await;
    //     for bucket in buckets {
    //         if committees.contains_key(&bucket) {
    //             continue;
    //         }
    //
    //         committees.insert(bucket, state.committees.get(&bucket).unwrap().clone());
    //     }
    //     Ok(committees)
    // }

    async fn get_committee_within_shard_range(
        &self,
        _epoch: Epoch,
        range: RangeInclusive<SubstateAddress>,
    ) -> Result<Committee<Self::Addr>, EpochManagerError> {
        let lock = self.state_lock().await;
        Ok(Committee::new(
            lock.validator_shards
                .iter()
                .filter(|(_, (_, s, _))| range.contains(s))
                .map(|(a, (_, _, pk))| (a.clone(), pk.clone()))
                .collect(),
        ))
    }

    async fn get_validator_node_by_public_key(
        &self,
        epoch: Epoch,
        public_key: &PublicKey,
    ) -> Result<ValidatorNode<Self::Addr>, EpochManagerError> {
        let lock = self.state_lock().await;
        let (address, (shard, shard_key, public_key)) = lock
            .validator_shards
            .iter()
            .find(|(_, (_, _, pk))| pk == public_key)
            .unwrap();

        Ok(ValidatorNode {
            address: address.clone(),
            public_key: public_key.clone(),
            shard_key: *shard_key,
            epoch,
            committee_shard: Some(*shard),
            fee_claim_public_key: PublicKey::default(),
        })
    }

    async fn get_base_layer_block_height(&self, _hash: FixedHash) -> Result<Option<u64>, EpochManagerError> {
        Ok(Some(self.inner.lock().await.current_block_info.0))
    }
    
    async fn get_network_committees(&self) -> Result<Vec<CommitteeShardInfo<Self::Addr>>, EpochManagerError> {
        let lock = self.state_lock().await;
        let commitees_lock = &lock.committees;
        let num_committees = commitees_lock.len().try_into().unwrap();

        let committees = commitees_lock.into_iter().map(|s| {
            let shard = s.0;
            CommitteeShardInfo {
                shard: *shard,
                substate_address_range: shard.to_substate_address_range(num_committees),
                validators: s.1.clone(),
            }
        }).collect();

        Ok(committees)
    }
}

#[derive(Debug, Clone)]
pub struct TestEpochManagerState {
    pub current_epoch: Epoch,
    pub current_block_info: (u64, FixedHash),
    pub is_epoch_active: bool,
    pub validator_shards: HashMap<TestAddress, (Shard, SubstateAddress, PublicKey)>,
    pub committees: HashMap<Shard, Committee<TestAddress>>,
    pub address_shard: HashMap<TestAddress, Shard>,
}

impl Default for TestEpochManagerState {
    fn default() -> Self {
        Self {
            current_epoch: Epoch(0),
            current_block_info: (0, FixedHash::default()),
            validator_shards: HashMap::new(),
            is_epoch_active: false,
            committees: HashMap::new(),
            address_shard: HashMap::new(),
        }
    }
}
