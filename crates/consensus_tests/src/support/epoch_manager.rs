//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex as StdMutex},
    time::Instant,
};

use ootle_byte_type::ToByteType;
use rand::seq::IteratorRandom;
use tari_common_types::types::FixedHash;
use tari_crypto::ristretto::RistrettoPublicKey;
use tari_epoch_manager::{EpochManagerError, EpochManagerEvent, EpochManagerReader};
use tari_ootle_common_types::{
    Epoch,
    ShardGroup,
    SubstateAddress,
    ToSubstateAddress,
    VersionedSubstateId,
    VotePower,
    committee::{Committee, CommitteeInfo},
    optional::Optional,
};
use tari_ootle_storage::{StorageError, global::models::ValidatorNode};
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;
use tokio::sync::{Mutex, MutexGuard, broadcast};

use crate::support::{
    TEST_NUM_PRESHARDS,
    TestVnDestination,
    address::TestAddress,
    helpers::random_substate_in_shard_group,
};

#[derive(Debug, Clone)]
pub struct TestEpochManager {
    inner: Arc<Mutex<TestEpochManagerState>>,
    our_validator_node: Option<ValidatorNode<TestAddress>>,
    tx_epoch_events: broadcast::Sender<EpochManagerEvent>,
    current_epoch: Epoch,
    /// Simulates a lagged base-layer oracle for this validator. When `Some(cap)`,
    /// `get_epoch_hash(e)` returns `NoEpochFound(e)` for `e > cap`.
    /// A fresh `Arc` is allocated by `clone_for` (so each validator has independent lag state),
    /// while the cheap `Clone` impl shares it with downstream consumers — the worker, outbound
    /// messaging, etc. — for the same validator.
    oracle_visible_epoch: Arc<StdMutex<Option<Epoch>>>,
    /// Caps the value returned by `current_epoch()` for this validator. When `Some(cap)`,
    /// `current_epoch().await` returns `min(state.current_epoch, cap)`. Unlike
    /// `oracle_visible_epoch`, this *does* lag the vote-time `em_epoch > current_epoch`
    /// check — used to reproduce the wedge where a validator no-votes the EOE block.
    oracle_current_epoch_cap: Arc<StdMutex<Option<Epoch>>>,
    /// Overrides the boundary hash this validator's oracle reports from `get_epoch_hash`. When
    /// `Some(hash)`, simulates a base-layer reorg that left this node on a different epoch-boundary
    /// block than its peers (the committee-split scenario). `None` = report the shared
    /// `last_epoch_hash`. Like the lag fields, a fresh `Arc` is allocated per validator by
    /// `clone_for`.
    oracle_epoch_hash_override: Arc<StdMutex<Option<FixedHash>>>,
}

impl TestEpochManager {
    pub fn new(tx_epoch_events: broadcast::Sender<EpochManagerEvent>) -> Self {
        Self {
            inner: Default::default(),
            our_validator_node: None,
            tx_epoch_events,
            current_epoch: Epoch(1),
            oracle_visible_epoch: Arc::new(StdMutex::new(None)),
            oracle_current_epoch_cap: Arc::new(StdMutex::new(None)),
            oracle_epoch_hash_override: Arc::new(StdMutex::new(None)),
        }
    }

    /// Cap this validator's oracle view to `epoch`. After this, `get_epoch_hash(e)` returns
    /// `NoEpochFound` for `e > epoch`. Mirrors a real validator whose base-layer scanner has
    /// not yet observed the new epoch.
    ///
    /// NB: the cap intentionally does *not* override `current_epoch()` — that lets the worker
    /// vote on the EOE block normally (so the chain progresses to a 3-chain commit on every
    /// node) while still tripping `process_end_of_epoch` when it tries to look up the next
    /// epoch's base-layer hash. That is exactly the failure mode the production bug exhibits
    /// once a node catches up via sync.
    pub fn set_oracle_visible_epoch(&self, epoch: Epoch) {
        *self.oracle_visible_epoch.lock().unwrap() = Some(epoch);
    }

    fn oracle_visible_epoch(&self) -> Option<Epoch> {
        *self.oracle_visible_epoch.lock().unwrap()
    }

    /// Cap the value returned by `current_epoch()` for this validator. After this,
    /// `current_epoch().await` returns `min(state.current_epoch, cap)`. Used by the
    /// EOE-no-vote wedge test: lagging this value makes the vote-time check
    /// `em_epoch > current_epoch` fail on this validator while peers (which read the
    /// shared `inner.current_epoch`) vote yes and commit the EOE without us.
    pub fn set_oracle_current_epoch_cap(&self, epoch: Epoch) {
        *self.oracle_current_epoch_cap.lock().unwrap() = Some(epoch);
    }

    /// Remove the `current_epoch()` cap for this validator.
    pub fn clear_oracle_current_epoch_cap(&self) {
        *self.oracle_current_epoch_cap.lock().unwrap() = None;
    }

    fn oracle_current_epoch_cap(&self) -> Option<Epoch> {
        *self.oracle_current_epoch_cap.lock().unwrap()
    }

    /// Override the boundary hash this validator's oracle reports from `get_epoch_hash`, for every
    /// epoch. Used to model a base-layer reorg that splits the committee across two boundary-block
    /// hashes: set different overrides on different validators and they will disagree on the next
    /// epoch's hash, just like the production divergence.
    pub fn set_oracle_epoch_hash(&self, hash: FixedHash) {
        *self.oracle_epoch_hash_override.lock().unwrap() = Some(hash);
    }

    fn oracle_epoch_hash_override(&self) -> Option<FixedHash> {
        *self.oracle_epoch_hash_override.lock().unwrap()
    }

    pub async fn set_current_epoch(&mut self, current_epoch: Epoch, shard_group: ShardGroup) -> &Self {
        self.current_epoch = current_epoch;
        {
            let mut lock = self.inner.lock().await;
            lock.current_epoch = current_epoch;
            lock.epoch_started = true;
        }

        let _ = self.tx_epoch_events.send(EpochManagerEvent::EpochChanged {
            epoch: current_epoch,
            registered_shard_group: Some(shard_group),
            activated_at: Instant::now(),
        });

        self
    }

    pub async fn state_lock(&self) -> MutexGuard<'_, TestEpochManagerState> {
        self.inner.lock().await
    }

    pub fn clone_for(
        &self,
        address: TestAddress,
        public_key: RistrettoPublicKey,
        shard_key: SubstateAddress,
        fee_claim_pk: RistrettoPublicKeyBytes,
    ) -> Self {
        let mut copy = self.clone();
        // Each validator gets its own lag state — sibling clones (worker, outbound, etc.) for
        // this same validator share via the cheap `Clone` impl.
        copy.oracle_visible_epoch = Arc::new(StdMutex::new(None));
        copy.oracle_current_epoch_cap = Arc::new(StdMutex::new(None));
        copy.oracle_epoch_hash_override = Arc::new(StdMutex::new(None));
        if let Some(our_validator_node) = self.our_validator_node.clone() {
            copy.our_validator_node = Some(ValidatorNode {
                address,
                public_key: public_key.to_byte_type(),
                shard_key,
                start_epoch: our_validator_node.start_epoch,
                end_epoch: None,
                fee_claim_public_key: fee_claim_pk,
                vote_power: our_validator_node.vote_power,
            });
        } else {
            copy.our_validator_node = Some(ValidatorNode {
                address,
                public_key: public_key.to_byte_type(),
                shard_key,
                start_epoch: Epoch(0),
                end_epoch: None,
                fee_claim_public_key: fee_claim_pk,
                vote_power: VotePower::of(1),
            });
        }
        copy
    }

    pub async fn add_committees(&self, committees: HashMap<ShardGroup, Committee<TestAddress>>) -> &Self {
        let mut state = self.state_lock().await;
        for (shard_group, committee) in committees {
            for member in committee.iter() {
                let substate_id = random_substate_in_shard_group(shard_group, TEST_NUM_PRESHARDS);
                let substate_id = VersionedSubstateId::new(substate_id, 0);
                state.validator_nodes.insert(
                    member.address.clone(),
                    (
                        ValidatorNode {
                            address: member.address.clone(),
                            public_key: member.public_key,
                            shard_key: substate_id.to_substate_address(),
                            start_epoch: Epoch(0),
                            end_epoch: None,
                            fee_claim_public_key: member.public_key,
                            vote_power: member.vote_power,
                        },
                        shard_group,
                    ),
                );
                state.address_shard.insert(member.address.clone(), shard_group);
            }

            state.committees.insert(shard_group, Arc::new(committee));
        }
        self
    }

    pub async fn set_claim_keys(&self, dest: TestVnDestination, claim_key: RistrettoPublicKey) -> &Self {
        let mut state = self.state_lock().await;
        let num_committees = state.committees.len() as u32;
        state.validator_nodes.iter_mut().for_each(|(address, (vn, sg))| {
            if dest.is_for(address, *sg, num_committees) {
                vn.fee_claim_public_key = claim_key.to_byte_type();
            }
        });
        self
    }

    pub async fn all_validators(&self) -> Vec<(ValidatorNode<TestAddress>, ShardGroup)> {
        self.state_lock().await.validator_nodes.values().cloned().collect()
    }

    pub fn get_current_epoch(&self) -> Epoch {
        self.current_epoch
    }

    pub async fn eviction_proofs(&self) -> Vec<tari_sidechain::EvictionProof> {
        self.state_lock().await.eviction_proofs.clone()
    }
}

impl EpochManagerReader for TestEpochManager {
    type Addr = TestAddress;

    fn subscribe(&self) -> broadcast::Receiver<EpochManagerEvent> {
        self.tx_epoch_events.subscribe()
    }

    async fn is_this_validator_registered_for_epoch(&self, epoch: Epoch) -> Result<bool, EpochManagerError> {
        if !self.state_lock().await.epoch_started {
            return Ok(false);
        }
        if self.current_epoch().await? < epoch {
            return Ok(false);
        }
        match self.get_local_committee_info(epoch).await {
            Ok(_) => Ok(true),
            Err(err) if err.is_not_registered_error() => Ok(false),
            Err(err) => Err(err),
        }
    }

    async fn get_committee_for_substate(
        &self,
        _epoch: Epoch,
        substate_address: SubstateAddress,
    ) -> Result<Arc<Committee<Self::Addr>>, EpochManagerError> {
        let state = self.state_lock().await;
        let shard_group = substate_address.to_shard_group(TEST_NUM_PRESHARDS, state.committees.len() as u32);
        Ok(state.committees[&shard_group].clone())
    }

    async fn get_our_validator_node(&self, _epoch: Epoch) -> Result<ValidatorNode<TestAddress>, EpochManagerError> {
        Ok(self.our_validator_node.clone().unwrap())
    }

    async fn get_all_validator_nodes(
        &self,
        _epoch: Epoch,
    ) -> Result<Vec<ValidatorNode<Self::Addr>>, EpochManagerError> {
        todo!()
    }

    async fn get_local_committee_info(&self, epoch: Epoch) -> Result<CommitteeInfo, EpochManagerError> {
        let our_vn = self.get_our_validator_node(epoch).await?;
        let num_committees = self.get_num_committees(epoch).await?;
        let sg = our_vn.shard_key.to_shard_group(TEST_NUM_PRESHARDS, num_committees);
        let num_shard_group_members = self
            .inner
            .lock()
            .await
            .committees
            .get(&sg)
            .map(|c| c.len())
            .unwrap_or(0);
        let total_power = self
            .inner
            .lock()
            .await
            .committees
            .get(&sg)
            .map(|c| c.total_power())
            .unwrap_or_else(VotePower::zero);

        Ok(CommitteeInfo::new(
            TEST_NUM_PRESHARDS,
            num_shard_group_members as u32,
            num_committees,
            sg,
            epoch,
            total_power,
        ))
    }

    async fn current_epoch(&self) -> Result<Epoch, EpochManagerError> {
        let actual = self.inner.lock().await.current_epoch;
        if let Some(cap) = self.oracle_current_epoch_cap() {
            return Ok(std::cmp::min(actual, cap));
        }
        Ok(actual)
    }

    async fn get_current_epoch_hash(&self) -> Result<FixedHash, EpochManagerError> {
        Ok(self.inner.lock().await.last_epoch_hash)
    }

    async fn get_epoch_hash(&self, epoch: Epoch) -> Result<FixedHash, EpochManagerError> {
        if let Some(cap) = self.oracle_visible_epoch() &&
            epoch > cap
        {
            return Err(EpochManagerError::NoEpochFound(epoch));
        }
        if let Some(hash) = self.oracle_epoch_hash_override() {
            return Ok(hash);
        }
        Ok(self.inner.lock().await.last_epoch_hash)
    }

    async fn get_num_committees(&self, _epoch: Epoch) -> Result<u32, EpochManagerError> {
        Ok(self.inner.lock().await.committees.len() as u32)
    }

    async fn get_committee_info_by_validator_address(
        &self,
        epoch: Epoch,
        address: &Self::Addr,
    ) -> Result<CommitteeInfo, EpochManagerError> {
        let state = self.state_lock().await;
        let (sg, committee) = state
            .committees
            .iter()
            .find(|(_, committee)| committee.contains(address))
            .unwrap_or_else(|| panic!("Validator {address} not found in any committee"));
        let num_committees = state.committees.len() as u32;
        let num_members = committee.len();
        let total_power = state
            .committees
            .get(sg)
            .map(|c| c.total_power())
            .unwrap_or_else(VotePower::zero);
        Ok(CommitteeInfo::new(
            TEST_NUM_PRESHARDS,
            num_members as u32,
            num_committees,
            *sg,
            epoch,
            total_power,
        ))
    }

    async fn get_committee_by_shard_group(
        &self,
        _epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<Arc<Committee<Self::Addr>>, EpochManagerError> {
        let state = self.state_lock().await;
        let Some(committee) = state.committees.get(&shard_group).cloned() else {
            panic!("Committee not found for shard group {}", shard_group);
        };

        Ok(committee)
    }

    async fn get_committees_overlapping_shard_group(
        &self,
        _epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<HashMap<ShardGroup, Committee<Self::Addr>>, EpochManagerError> {
        let state = self.state_lock().await;
        let mut committees = HashMap::new();
        for (sg, committee) in &state.committees {
            if sg.overlaps_shard_group(&shard_group) {
                committees.insert(*sg, (**committee).clone());
            }
        }
        Ok(committees)
    }

    async fn get_committee_info_for_substate(
        &self,
        epoch: Epoch,
        substate_address: SubstateAddress,
    ) -> Result<CommitteeInfo, EpochManagerError> {
        let num_committees = self.get_num_committees(epoch).await?;
        let sg = substate_address.to_shard_group(TEST_NUM_PRESHARDS, num_committees);
        let num_members = self
            .inner
            .lock()
            .await
            .committees
            .get(&sg)
            .map(|c| c.len())
            .unwrap_or(0);
        let total_power = self
            .inner
            .lock()
            .await
            .committees
            .get(&sg)
            .map(|c| c.total_power())
            .unwrap_or_else(VotePower::zero);

        Ok(CommitteeInfo::new(
            TEST_NUM_PRESHARDS,
            num_members as u32,
            num_committees,
            sg,
            epoch,
            total_power,
        ))
    }

    async fn get_validator_node_by_public_key(
        &self,
        _epoch: Epoch,
        public_key: RistrettoPublicKeyBytes,
    ) -> Result<ValidatorNode<Self::Addr>, EpochManagerError> {
        let lock = self.state_lock().await;
        let (vn, _) = lock
            .validator_nodes
            .values()
            .find(|(vn, _)| vn.public_key == public_key)
            .unwrap_or_else(|| panic!("Validator node not found for public key {}", public_key));

        Ok(ValidatorNode {
            address: vn.address.clone(),
            public_key: vn.public_key,
            shard_key: vn.shard_key,
            start_epoch: vn.start_epoch,
            end_epoch: vn.end_epoch,
            fee_claim_public_key: vn.fee_claim_public_key,
            vote_power: vn.vote_power,
        })
    }

    async fn wait_for_initial_scanning_to_complete(&self) -> Result<(), EpochManagerError> {
        // Scanning is not relevant to tests
        Ok(())
    }

    async fn add_intent_to_evict_validator(
        &self,
        proof: tari_sidechain::EvictionProof,
    ) -> Result<(), EpochManagerError> {
        let mut state = self.state_lock().await;
        state.eviction_proofs.push(proof);
        Ok(())
    }

    async fn get_random_committee_member(
        &self,
        _epoch: Epoch,
        shard_group: Option<ShardGroup>,
        excluding: HashSet<Self::Addr>,
    ) -> Result<ValidatorNode<Self::Addr>, EpochManagerError> {
        let state = self.state_lock().await;
        let vn = match shard_group {
            Some(shard_group) => state
                .validator_nodes
                .values()
                .filter(|(_, sg)| *sg == shard_group)
                .map(|(vn, _)| vn)
                .find(|vn| !excluding.contains(&vn.address))
                .ok_or_else(|| {
                    EpochManagerError::StorageError(StorageError::NotFound {
                        item: "validator_nodes",
                        key: format!("in shard group {shard_group}"),
                    })
                })?,
            None => state
                .validator_nodes
                .values()
                .choose(&mut rand::rng())
                .map(|(vn, _)| vn)
                .expect("No committees?"),
        };

        Ok(vn.clone())
    }

    async fn get_committee_info(
        &self,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<CommitteeInfo, EpochManagerError> {
        let state = self.state_lock().await;
        let committee = state.committees.get(&shard_group).ok_or_else(|| {
            EpochManagerError::StorageError(StorageError::NotFound {
                item: "committee",
                key: format!("for shard group {shard_group}"),
            })
        })?;

        let num_members = committee.len();
        let total_power = committee.total_power();

        Ok(CommitteeInfo::new(
            TEST_NUM_PRESHARDS,
            num_members as u32,
            state.committees.len() as u32,
            shard_group,
            epoch,
            total_power,
        ))
    }

    async fn lock_epoch(&self, _epoch: Epoch) -> Result<(), EpochManagerError> {
        Ok(())
    }

    async fn is_within_epoch_end_spread(&self, _current_epoch: Epoch) -> Result<bool, EpochManagerError> {
        Ok(false)
    }

    async fn get_observed_epoch_hash(&self, epoch: Epoch) -> Result<Option<FixedHash>, EpochManagerError> {
        self.get_epoch_hash(epoch).await.optional()
    }

    async fn get_birthday_epoch(&self) -> Result<Option<Epoch>, EpochManagerError> {
        Ok(Some(Epoch(0)))
    }
}

#[derive(Debug, Clone)]
pub struct TestEpochManagerState {
    pub current_epoch: Epoch,
    /// Validators must not leave the idle state until the test calls `set_current_epoch`, otherwise a
    /// validator spawned early runs its pacemaker against peers that do not exist yet.
    pub epoch_started: bool,
    pub last_epoch_hash: FixedHash,
    #[allow(clippy::type_complexity)]
    pub validator_nodes: HashMap<TestAddress, (ValidatorNode<TestAddress>, ShardGroup)>,
    pub committees: HashMap<ShardGroup, Arc<Committee<TestAddress>>>,
    pub address_shard: HashMap<TestAddress, ShardGroup>,
    pub eviction_proofs: Vec<tari_sidechain::EvictionProof>,
}

impl Default for TestEpochManagerState {
    fn default() -> Self {
        Self {
            current_epoch: Epoch(1),
            epoch_started: false,
            last_epoch_hash: FixedHash::default(),
            validator_nodes: HashMap::new(),
            committees: HashMap::new(),
            address_shard: HashMap::new(),
            eviction_proofs: Vec::new(),
        }
    }
}
