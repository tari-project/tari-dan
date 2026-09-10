//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, atomic::AtomicU64},
};

use tari_common_types::types::FixedHash;
use tari_ootle_common_types::{
    Epoch,
    NodeAddressable,
    ShardGroup,
    SubstateAddress,
    VotePower,
    committee::{Committee, CommitteeInfo},
};
use tari_ootle_storage::global::models::ValidatorNode;
use tari_sidechain::EvictionProof;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::{
    EpochManagerEvent,
    error::EpochManagerError,
    service::{CommitteeCache, NetworkDescription, types::EpochManagerRequest},
    traits::{EpochManagerReader, EpochManagerWriter},
};

#[derive(Clone, Debug)]
pub struct EpochManagerHandle<TAddr> {
    tx_request: mpsc::Sender<EpochManagerRequest<TAddr>>,
    current_epoch: Arc<AtomicU64>,
    events: broadcast::WeakSender<EpochManagerEvent>,
    committee_cache: CommitteeCache<TAddr>,
}

impl<TAddr: NodeAddressable> EpochManagerHandle<TAddr> {
    pub fn new(
        tx_request: mpsc::Sender<EpochManagerRequest<TAddr>>,
        events: broadcast::WeakSender<EpochManagerEvent>,
        current_epoch: Arc<AtomicU64>,
        committee_cache: CommitteeCache<TAddr>,
    ) -> Self {
        Self {
            tx_request,
            events,
            current_epoch,
            committee_cache,
        }
    }

    pub async fn get_fee_claim_public_key(&self) -> Result<Option<RistrettoPublicKeyBytes>, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetFeeClaimPublicKey { reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    pub async fn is_initial_scanning_complete(&self) -> Result<bool, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::IsInitialScanningComplete { reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    /// Non-async and infallible version of `current_epoch`. TODO: change the trait to be non-async and infallible
    pub fn get_current_epoch(&self) -> Epoch {
        Epoch(self.current_epoch.load(std::sync::atomic::Ordering::SeqCst))
    }

    pub fn is_closed(&self) -> bool {
        self.tx_request.is_closed()
    }

    pub async fn get_network_description(&self) -> Result<NetworkDescription, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetNetworkDescription { reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }
}

impl<TAddr: NodeAddressable> EpochManagerWriter for EpochManagerHandle<TAddr> {
    async fn add_validator_node_registration(
        &mut self,
        activation_epoch: Epoch,
        validator_public_key: RistrettoPublicKeyBytes,
        claim_public_key: RistrettoPublicKeyBytes,
        shard_key: SubstateAddress,
        power: VotePower,
    ) -> Result<(), EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::AddValidatorNodeRegistration {
                activation_epoch,
                validator_public_key,
                claim_public_key,
                power,
                shard_key,
                reply: tx,
            })
            .await
            .map_err(|_| EpochManagerError::SendError)?;
        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn deactivate_validator_node(
        &mut self,
        public_key: RistrettoPublicKeyBytes,
        deactivation_epoch: Epoch,
    ) -> Result<(), EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::DeactivateValidatorNode {
                public_key,
                deactivation_epoch,
                reply: tx,
            })
            .await
            .map_err(|_| EpochManagerError::SendError)?;
        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }
}

impl<TAddr: NodeAddressable> EpochManagerReader for EpochManagerHandle<TAddr> {
    type Addr = TAddr;

    fn subscribe(&self) -> broadcast::Receiver<EpochManagerEvent> {
        let sender = self.events.upgrade().unwrap_or_else(|| {
            // Should the sender be closed (upgrade() returns None), create a "dummy" channel that will
            // immediately close. This is more in-line with what you would expect from the api.
            broadcast::Sender::new(1)
        });
        sender.subscribe()
    }

    async fn wait_for_initial_scanning_to_complete(&self) -> Result<(), EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::WaitForInitialScanningToComplete { reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn get_all_validator_nodes(&self, epoch: Epoch) -> Result<Vec<ValidatorNode<TAddr>>, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetValidatorNodesPerEpoch { epoch, reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn get_committee_for_substate(
        &self,
        epoch: Epoch,
        substate_address: SubstateAddress,
    ) -> Result<Arc<Committee<Self::Addr>>, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetCommitteeForSubstate {
                epoch,
                substate_address,
                reply: tx,
            })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn get_validator_node_by_public_key(
        &self,
        epoch: Epoch,
        public_key: RistrettoPublicKeyBytes,
    ) -> Result<ValidatorNode<Self::Addr>, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetValidatorNodeByPublicKey {
                epoch,
                public_key,
                reply: tx,
            })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn get_our_validator_node(&self, epoch: Epoch) -> Result<ValidatorNode<Self::Addr>, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetOurValidatorNode { epoch, reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn get_local_committee_info(&self, epoch: Epoch) -> Result<CommitteeInfo, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetLocalCommitteeInfo { epoch, reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn get_committee_info(
        &self,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<CommitteeInfo, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetCommitteeInfo {
                epoch,
                shard_group,
                reply: tx,
            })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn get_committee_info_for_substate(
        &self,
        epoch: Epoch,
        substate_address: SubstateAddress,
    ) -> Result<CommitteeInfo, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetCommitteeInfoForSubstate {
                epoch,
                substate_address,
                reply: tx,
            })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn get_committee_info_by_validator_address(
        &self,
        epoch: Epoch,
        address: &TAddr,
    ) -> Result<CommitteeInfo, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetCommitteeInfoByAddress {
                epoch,
                address: address.clone(),
                reply: tx,
            })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn current_epoch(&self) -> Result<Epoch, EpochManagerError> {
        Ok(Epoch(self.current_epoch.load(std::sync::atomic::Ordering::SeqCst)))
    }

    async fn get_current_epoch_hash(&self) -> Result<FixedHash, EpochManagerError> {
        // Resolve the current epoch inside the service rather than reading the shared atomic here.
        // Reading the epoch and then requesting its hash is racy: the service may advance the
        // epoch in between, and the previously-current epoch may have no persisted row (epoch 0 on
        // a fresh chain never does), turning a "current hash" read into a spurious NoEpochFound.
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetCurrentEpochHash { reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn get_epoch_hash(&self, epoch: Epoch) -> Result<FixedHash, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetEpochHash { epoch, reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn get_num_committees(&self, epoch: Epoch) -> Result<u32, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetNumCommittees { epoch, reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn get_committee_by_shard_group(
        &self,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<Arc<Committee<Self::Addr>>, EpochManagerError> {
        self.committee_cache
            .get_or_try_init((epoch, shard_group), || async {
                let (tx, rx) = oneshot::channel();
                self.tx_request
                    .send(EpochManagerRequest::GetCommitteeForShardGroup {
                        epoch,
                        shard_group,
                        reply: tx,
                    })
                    .await
                    .map_err(|_| EpochManagerError::SendError)?;
                rx.await.map_err(|_| EpochManagerError::ReceiveError)?
            })
            .await
    }

    async fn get_committees_overlapping_shard_group(
        &self,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<HashMap<ShardGroup, Committee<Self::Addr>>, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetCommitteesOverlappingShardGroup {
                epoch,
                shard_group,
                reply: tx,
            })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn add_intent_to_evict_validator(&self, proof: EvictionProof) -> Result<(), EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::AddIntentToEvictValidator {
                proof: Box::new(proof),
                reply: tx,
            })
            .await
            .map_err(|_| EpochManagerError::SendError)?;
        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn get_random_committee_member(
        &self,
        epoch: Epoch,
        shard_group: Option<ShardGroup>,
        excluding: HashSet<TAddr>,
    ) -> Result<ValidatorNode<TAddr>, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetRandomCommitteeMemberFromShardGroup {
                epoch,
                shard_group,
                excluding,
                reply: tx,
            })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn lock_epoch(&self, epoch: Epoch) -> Result<(), EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::LockEpoch { epoch, reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;
        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn is_within_epoch_end_spread(&self, current_epoch: Epoch) -> Result<bool, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::IsWithinEpochEndSpread {
                current_epoch,
                reply: tx,
            })
            .await
            .map_err(|_| EpochManagerError::SendError)?;
        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn get_observed_epoch_hash(&self, epoch: Epoch) -> Result<Option<FixedHash>, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetObservedEpochHash { epoch, reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;
        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn get_birthday_epoch(&self) -> Result<Option<Epoch>, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetBirthdayEpoch { reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;
        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }
}
