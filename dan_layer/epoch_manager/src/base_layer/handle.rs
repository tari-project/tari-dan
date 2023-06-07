//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::RangeInclusive;

use async_trait::async_trait;
use tari_base_node_client::types::BaseLayerConsensusConstants;
use tari_common_types::types::FixedHash;
use tari_comms::types::CommsPublicKey;
use tari_core::{transactions::transaction_components::ValidatorNodeRegistration, ValidatorNodeBMT};
use tari_dan_common_types::{Epoch, ShardId};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::{
    base_layer::{
        error::EpochManagerError,
        types::{EpochManagerEvent, EpochManagerRequest},
    },
    traits::EpochManager,
    validator_node::ValidatorNode,
    Committee, ShardCommitteeAllocation,
};

#[derive(Clone, Debug)]
pub struct EpochManagerHandle {
    tx_request: mpsc::Sender<EpochManagerRequest>,
}

impl EpochManagerHandle {
    pub fn new(tx_request: mpsc::Sender<EpochManagerRequest>) -> Self {
        Self { tx_request }
    }

    pub async fn update_epoch(&self, block_height: u64, block_hash: FixedHash) -> Result<(), EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::UpdateEpoch {
                block_height,
                block_hash,
                reply: tx,
            })
            .await
            .map_err(|_| EpochManagerError::SendError)?;
        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    pub async fn get_base_layer_consensus_constants(&self) -> Result<BaseLayerConsensusConstants, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetBaseLayerConsensusConstants { reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;
        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    pub async fn last_registration_epoch(&self) -> Result<Option<Epoch>, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::LastRegistrationEpoch { reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;
        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    pub async fn update_last_registration_epoch(&self, epoch: Epoch) -> Result<(), EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::UpdateLastRegistrationEpoch { epoch, reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    /// Returns the number of epochs remaining for the current registration if registered, otherwise None
    pub async fn remaining_registration_epochs(&self) -> Result<Option<Epoch>, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::RemainingRegistrationEpochs { reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;
        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    pub async fn add_validator_node_registration(
        &self,
        block_height: u64,
        registration: ValidatorNodeRegistration,
    ) -> Result<(), EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::AddValidatorNodeRegistration {
                block_height,
                registration,
                reply: tx,
            })
            .await
            .map_err(|_| EpochManagerError::SendError)?;
        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    pub async fn subscribe(&self) -> Result<broadcast::Receiver<EpochManagerEvent>, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::Subscribe { reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;
        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }
}

#[async_trait]
impl EpochManager<CommsPublicKey> for EpochManagerHandle {
    type Error = EpochManagerError;

    async fn current_epoch(&self) -> Result<Epoch, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::CurrentEpoch { reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn current_block_height(&self) -> Result<u64, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::CurrentBlockHeight { reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn get_validator_node(
        &self,
        epoch: Epoch,
        addr: CommsPublicKey,
    ) -> Result<ValidatorNode<CommsPublicKey>, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetValidatorNode { epoch, addr, reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn is_epoch_valid(&self, epoch: Epoch) -> Result<bool, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::IsEpochValid { epoch, reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn get_committees(
        &self,
        epoch: Epoch,
        shards: &[ShardId],
    ) -> Result<Vec<ShardCommitteeAllocation<CommsPublicKey>>, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetCommittees {
                epoch,
                shards: shards.to_vec(),
                reply: tx,
            })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn get_committee(
        &self,
        epoch: Epoch,
        shard: ShardId,
    ) -> Result<Committee<CommsPublicKey>, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetCommittee {
                epoch,
                shard,
                reply: tx,
            })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn get_committee_for_shard_range(
        &self,
        epoch: Epoch,
        shard_range: RangeInclusive<ShardId>,
    ) -> Result<Committee<CommsPublicKey>, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetCommitteeForShardRange {
                epoch,
                shard_range,
                reply: tx,
            })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn is_validator_in_committee_for_current_epoch(
        &self,
        shard: ShardId,
        identity: CommsPublicKey,
    ) -> Result<bool, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::IsValidatorInCommitteeForCurrentEpoch {
                shard,
                identity,
                reply: tx,
            })
            .await
            .map_err(|_| EpochManagerError::SendError)?;
        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    /// Filters out from the available_shards, returning the ShardIds for committees for each available_shard that
    /// `for_addr` is part of.
    async fn filter_to_local_shards(
        &self,
        epoch: Epoch,
        for_addr: &CommsPublicKey,
        available_shards: &[ShardId],
    ) -> Result<Vec<ShardId>, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::FilterToLocalShards {
                epoch,
                for_addr: for_addr.clone(),
                available_shards: available_shards.to_vec(),
                reply: tx,
            })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn get_validator_nodes_per_epoch(
        &self,
        epoch: Epoch,
    ) -> Result<Vec<ValidatorNode<CommsPublicKey>>, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetValidatorNodesPerEpoch { epoch, reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn get_validator_node_bmt(&self, epoch: Epoch) -> Result<ValidatorNodeBMT, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetValidatorNodeBMT { epoch, reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn get_validator_node_merkle_root(&self, epoch: Epoch) -> Result<Vec<u8>, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetValidatorNodeMerkleRoot { epoch, reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    async fn get_local_shard_range(
        &self,
        epoch: Epoch,
        addr: CommsPublicKey,
    ) -> Result<RangeInclusive<ShardId>, Self::Error> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::GetLocalShardRange {
                epoch,
                for_addr: addr,
                reply: tx,
            })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }

    /// Note: this awaits until scanning is complete.
    async fn notify_scanning_complete(&self) -> Result<(), EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send(EpochManagerRequest::NotifyScanningComplete { reply: tx })
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        rx.await.map_err(|_| EpochManagerError::ReceiveError)?
    }
}
