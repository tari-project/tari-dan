//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{
    cmp,
    collections::{HashMap, HashSet},
    num::NonZeroU32,
};

use log::*;
use tari_base_node_client::{grpc::GrpcBaseNodeClient, types::BaseLayerConsensusConstants, BaseNodeClient};
use tari_common_types::types::{FixedHash, PublicKey};
use tari_core::{blocks::BlockHeader, transactions::transaction_components::ValidatorNodeRegistration};
use tari_dan_common_types::{
    committee::{Committee, CommitteeInfo},
    optional::Optional,
    shard::Shard,
    DerivableFromPublicKey,
    Epoch,
    NodeAddressable,
    SubstateAddress,
};
use tari_dan_storage::global::{models::ValidatorNode, DbBaseLayerBlockInfo, DbEpoch, GlobalDb, MetadataKey};
use tari_dan_storage_sqlite::global::SqliteGlobalDbAdapter;
use tari_utilities::{byte_array::ByteArray, hex::Hex};
use tokio::sync::{broadcast, oneshot};

use crate::{base_layer::config::EpochManagerConfig, error::EpochManagerError, EpochManagerEvent};

const LOG_TARGET: &str = "tari::dan::epoch_manager::base_layer";

pub struct BaseLayerEpochManager<TGlobalStore, TBaseNodeClient> {
    global_db: GlobalDb<TGlobalStore>,
    base_node_client: TBaseNodeClient,
    config: EpochManagerConfig,
    current_epoch: Epoch,
    current_block_info: (u64, FixedHash),
    last_block_of_current_epoch: FixedHash,
    tx_events: broadcast::Sender<EpochManagerEvent>,
    waiting_for_scanning_complete: Vec<oneshot::Sender<Result<(), EpochManagerError>>>,
    node_public_key: PublicKey,
    current_shard_key: Option<SubstateAddress>,
    base_layer_consensus_constants: Option<BaseLayerConsensusConstants>,
    is_initial_base_layer_sync_complete: bool,
}

impl<TAddr: NodeAddressable + DerivableFromPublicKey>
    BaseLayerEpochManager<SqliteGlobalDbAdapter<TAddr>, GrpcBaseNodeClient>
{
    pub fn new(
        config: EpochManagerConfig,
        global_db: GlobalDb<SqliteGlobalDbAdapter<TAddr>>,
        base_node_client: GrpcBaseNodeClient,
        tx_events: broadcast::Sender<EpochManagerEvent>,
        node_public_key: PublicKey,
    ) -> Self {
        Self {
            global_db,
            base_node_client,
            config,
            current_epoch: Epoch(0),
            current_block_info: (0, Default::default()),
            last_block_of_current_epoch: Default::default(),
            waiting_for_scanning_complete: Vec::new(),
            tx_events,
            node_public_key,
            current_shard_key: None,
            base_layer_consensus_constants: None,
            is_initial_base_layer_sync_complete: false,
        }
    }

    pub async fn load_initial_state(&mut self) -> Result<(), EpochManagerError> {
        info!(target: LOG_TARGET, "Loading base layer constants");
        self.refresh_base_layer_consensus_constants().await?;

        info!(target: LOG_TARGET, "Retrieving current epoch and block info from database");
        let mut tx = self.global_db.create_transaction()?;
        let mut metadata = self.global_db.metadata(&mut tx);
        self.current_epoch = metadata
            .get_metadata(MetadataKey::EpochManagerCurrentEpoch)?
            .unwrap_or(Epoch(0));
        self.current_shard_key = metadata.get_metadata(MetadataKey::EpochManagerCurrentShardKey)?;
        self.current_block_info = metadata
            .get_metadata(MetadataKey::EpochManagerCurrentBlockHeight)?
            .unwrap_or((0, Default::default()));
        self.last_block_of_current_epoch = metadata
            .get_metadata(MetadataKey::EpochManagerLastBlockOfCurrentEpoch)?
            .unwrap_or(Default::default());
        Ok(())
    }

    pub async fn update_epoch(&mut self, block_height: u64, block_hash: FixedHash) -> Result<(), EpochManagerError> {
        let base_layer_constants = self.base_node_client.get_consensus_constants(block_height).await?;
        let epoch = base_layer_constants.height_to_epoch(block_height);
        self.add_base_layer_block_info(block_height, block_hash)?;
        let previous_block = self.current_block_info.1;
        self.update_current_block_info(block_height, block_hash)?;
        if self.current_epoch >= epoch {
            // no need to update the epoch
            return Ok(());
        }
        // When epoch is changing we store the last block of current epoch for the EpochEvents
        self.update_last_block_of_current_epoch(previous_block)?;

        info!(target: LOG_TARGET, "🌟 A new epoch {} is upon us", epoch);
        // extract and store in database the MMR of the epoch's validator nodes
        let epoch_header = self.base_node_client.get_header_by_hash(block_hash).await?;

        // persist the epoch data including the validator node set
        self.insert_current_epoch(epoch, epoch_header)?;
        self.update_base_layer_consensus_constants(base_layer_constants)?;
        self.assign_validators_for_epoch(epoch)?;

        // Only publish an epoch change event if we have synced the base layer (see on_scanning_complete)
        if self.is_initial_base_layer_sync_complete {
            self.publish_event(EpochManagerEvent::EpochChanged(epoch));
        }

        Ok(())
    }

    fn assign_validators_for_epoch(&mut self, epoch: Epoch) -> Result<(), EpochManagerError> {
        let mut tx = self.global_db.create_transaction()?;
        let mut validator_nodes = self.global_db.validator_nodes(&mut tx);

        let vns = validator_nodes.get_all_within_epoch(epoch, self.config.validator_node_sidechain_id.as_ref())?;

        let num_committees = calculate_num_committees(vns.len() as u64, self.config.committee_size);

        for vn in &vns {
            validator_nodes.set_committee_shard(
                vn.shard_key,
                vn.shard_key.to_shard(num_committees),
                self.config.validator_node_sidechain_id.as_ref(),
                epoch,
            )?;
        }
        tx.commit()?;
        if let Some(vn) = vns.iter().find(|vn| vn.public_key == self.node_public_key) {
            self.publish_event(EpochManagerEvent::ThisValidatorIsRegistered {
                epoch,
                shard_key: vn.shard_key,
            });
        }

        Ok(())
    }

    pub async fn get_base_layer_consensus_constants(
        &mut self,
    ) -> Result<&BaseLayerConsensusConstants, EpochManagerError> {
        if let Some(ref constants) = self.base_layer_consensus_constants {
            return Ok(constants);
        }

        self.refresh_base_layer_consensus_constants().await?;

        Ok(self
            .base_layer_consensus_constants
            .as_ref()
            .expect("update_base_layer_consensus_constants did not set constants"))
    }

    async fn refresh_base_layer_consensus_constants(&mut self) -> Result<(), EpochManagerError> {
        let tip = self.base_node_client.get_tip_info().await?;
        let dan_tip = tip
            .height_of_longest_chain
            .saturating_sub(self.config.base_layer_confirmations);

        let constants = self.base_node_client.get_consensus_constants(dan_tip).await?;
        self.update_base_layer_consensus_constants(constants)?;
        Ok(())
    }

    pub async fn add_validator_node_registration(
        &mut self,
        block_height: u64,
        registration: ValidatorNodeRegistration,
    ) -> Result<(), EpochManagerError> {
        if registration.sidechain_id() != self.config.validator_node_sidechain_id.as_ref() {
            return Err(EpochManagerError::ValidatorNodeRegistrationSidechainIdMismatch {
                expected: self.config.validator_node_sidechain_id.as_ref().map(|v| v.to_hex()),
                actual: registration.sidechain_id().map(|v| v.to_hex()),
            });
        }
        let constants = self.get_base_layer_consensus_constants().await?;
        let next_epoch = constants.height_to_epoch(block_height) + Epoch(1);
        let next_epoch_height = constants.epoch_to_height(next_epoch);
        let validator_node_expiry = constants.validator_node_registration_expiry;

        let shard_key = self
            .base_node_client
            .get_shard_key(next_epoch_height, registration.public_key())
            .await?
            .ok_or_else(|| EpochManagerError::ShardKeyNotFound {
                public_key: registration.public_key().clone(),
                block_height,
            })?;

        let mut tx = self.global_db.create_transaction()?;
        info!(target: LOG_TARGET, "Registering validator node for epoch {}", next_epoch);
        self.global_db.validator_nodes(&mut tx).insert_validator_node(
            TAddr::derive_from_public_key(registration.public_key()),
            registration.public_key().clone(),
            shard_key,
            block_height,
            next_epoch,
            next_epoch + Epoch(validator_node_expiry),
            registration.claim_public_key().clone(),
            registration.sidechain_id().cloned(),
        )?;

        if *registration.public_key() == self.node_public_key {
            let mut metadata = self.global_db.metadata(&mut tx);
            metadata.set_metadata(MetadataKey::EpochManagerCurrentShardKey, &shard_key)?;
            let last_registration_epoch = metadata
                .get_metadata::<Epoch>(MetadataKey::EpochManagerLastEpochRegistration)?
                .unwrap_or(Epoch(0));
            if last_registration_epoch < next_epoch {
                metadata.set_metadata(MetadataKey::EpochManagerLastEpochRegistration, &next_epoch)?;
            }
            self.current_shard_key = Some(shard_key);
            info!(
                target: LOG_TARGET,
                "📋️ This validator node is registered for epoch {}, shard key: {} ", next_epoch, shard_key
            );
        }

        tx.commit()?;

        Ok(())
    }

    fn insert_current_epoch(&mut self, epoch: Epoch, header: BlockHeader) -> Result<(), EpochManagerError> {
        let epoch_height = epoch.0;
        let db_epoch = DbEpoch {
            epoch: epoch_height,
            validator_node_mr: header.validator_node_mr.to_vec(),
        };

        let mut tx = self.global_db.create_transaction()?;

        self.global_db.epochs(&mut tx).insert_epoch(db_epoch)?;
        self.global_db
            .metadata(&mut tx)
            .set_metadata(MetadataKey::EpochManagerCurrentEpoch, &epoch)?;

        tx.commit()?;
        self.current_epoch = epoch;
        Ok(())
    }

    fn update_base_layer_consensus_constants(
        &mut self,
        base_layer_constants: BaseLayerConsensusConstants,
    ) -> Result<(), EpochManagerError> {
        let mut tx = self.global_db.create_transaction()?;
        self.global_db
            .metadata(&mut tx)
            .set_metadata(MetadataKey::BaseLayerConsensusConstants, &base_layer_constants)?;
        tx.commit()?;
        self.base_layer_consensus_constants = Some(base_layer_constants);
        Ok(())
    }

    pub fn add_base_layer_block_info(
        &mut self,
        block_height: u64,
        block_hash: FixedHash,
    ) -> Result<(), EpochManagerError> {
        let mut tx = self.global_db.create_transaction()?;
        self.global_db
            .base_layer_hashes(&mut tx)
            .insert_base_layer_block_info(DbBaseLayerBlockInfo {
                hash: block_hash,
                height: block_height,
            })?;
        tx.commit()?;
        Ok(())
    }

    fn update_current_block_info(&mut self, block_height: u64, block_hash: FixedHash) -> Result<(), EpochManagerError> {
        let mut tx = self.global_db.create_transaction()?;
        self.global_db
            .metadata(&mut tx)
            .set_metadata(MetadataKey::EpochManagerCurrentBlockHeight, &(block_height, block_hash))?;
        tx.commit()?;
        self.current_block_info = (block_height, block_hash);
        Ok(())
    }

    fn update_last_block_of_current_epoch(&mut self, block_hash: FixedHash) -> Result<(), EpochManagerError> {
        let mut tx = self.global_db.create_transaction()?;
        self.global_db
            .metadata(&mut tx)
            .set_metadata(MetadataKey::EpochManagerLastBlockOfCurrentEpoch, &block_hash)?;
        tx.commit()?;
        self.last_block_of_current_epoch = block_hash;
        Ok(())
    }

    pub fn current_epoch(&self) -> Epoch {
        self.current_epoch
    }

    pub fn current_block_info(&self) -> (u64, FixedHash) {
        self.current_block_info
    }

    pub fn last_block_of_current_epoch(&self) -> FixedHash {
        self.last_block_of_current_epoch
    }

    pub async fn is_last_block_of_epoch(&mut self, block_height: u64) -> Result<bool, EpochManagerError> {
        let base_layer_constants_now = self.base_node_client.get_consensus_constants(block_height).await?;
        let base_layer_constants_next_block = self.base_node_client.get_consensus_constants(block_height + 1).await?;
        Ok(base_layer_constants_now.height_to_epoch(block_height) !=
            base_layer_constants_next_block.height_to_epoch(block_height + 1))
    }

    pub fn get_validator_node_by_public_key(
        &self,
        epoch: Epoch,
        public_key: &PublicKey,
    ) -> Result<Option<ValidatorNode<TAddr>>, EpochManagerError> {
        trace!(
            target: LOG_TARGET,
            "get_validator_node: epoch {} with public key {}", epoch, public_key,
        );
        let mut tx = self.global_db.create_transaction()?;
        let vn = self
            .global_db
            .validator_nodes(&mut tx)
            .get_by_public_key(epoch, public_key, self.config.validator_node_sidechain_id.as_ref())
            .optional()?;

        Ok(vn)
    }

    pub fn get_validator_node_by_address(
        &self,
        epoch: Epoch,
        address: &TAddr,
    ) -> Result<Option<ValidatorNode<TAddr>>, EpochManagerError> {
        trace!(
            target: LOG_TARGET,
            "get_validator_node: epoch {} with public key {}", epoch, address,
        );
        let mut tx = self.global_db.create_transaction()?;
        let vn = self
            .global_db
            .validator_nodes(&mut tx)
            .get_by_address(epoch, address, self.config.validator_node_sidechain_id.as_ref())
            .optional()?;

        Ok(vn)
    }

    pub fn get_many_validator_nodes(
        &self,
        epoch_validators: Vec<(Epoch, PublicKey)>,
    ) -> Result<HashMap<(Epoch, PublicKey), ValidatorNode<TAddr>>, EpochManagerError> {
        let mut tx = self.global_db.create_transaction()?;
        #[allow(clippy::mutable_key_type)]
        let mut validators = HashMap::with_capacity(epoch_validators.len());

        for (epoch, public_key) in epoch_validators {
            let vn = self
                .global_db
                .validator_nodes(&mut tx)
                .get_by_public_key(epoch, &public_key, self.config.validator_node_sidechain_id.as_ref())
                .optional()?
                .ok_or_else(|| EpochManagerError::ValidatorNodeNotRegistered {
                    address: public_key.to_string(),
                    epoch,
                })?;

            validators.insert((epoch, public_key), vn);
        }

        Ok(validators)
    }

    pub fn last_registration_epoch(&self) -> Result<Option<Epoch>, EpochManagerError> {
        let mut tx = self.global_db.create_transaction()?;
        let mut metadata = self.global_db.metadata(&mut tx);
        let last_registration_epoch = metadata.get_metadata(MetadataKey::EpochManagerLastEpochRegistration)?;
        Ok(last_registration_epoch)
    }

    pub fn update_last_registration_epoch(&self, epoch: Epoch) -> Result<(), EpochManagerError> {
        let mut tx = self.global_db.create_transaction()?;
        self.global_db
            .metadata(&mut tx)
            .set_metadata(MetadataKey::EpochManagerLastEpochRegistration, &epoch)?;
        tx.commit()?;
        Ok(())
    }

    pub fn is_epoch_valid(&self, epoch: Epoch) -> bool {
        let current_epoch = self.current_epoch();
        // Allow for 10 epochs behind. TODO: Properly define a "valid" epoch
        epoch.as_u64() >= current_epoch.as_u64().saturating_sub(10) && epoch.as_u64() <= current_epoch.as_u64()
    }

    pub fn get_committees(&self, epoch: Epoch) -> Result<HashMap<Shard, Committee<TAddr>>, EpochManagerError> {
        let mut tx = self.global_db.create_transaction()?;
        let mut validator_node_db = self.global_db.validator_nodes(&mut tx);
        Ok(validator_node_db.get_committees(epoch, self.config.validator_node_sidechain_id.as_ref())?)
    }

    pub fn get_committee_info_by_validator_address(
        &self,
        epoch: Epoch,
        validator_addr: TAddr,
    ) -> Result<CommitteeInfo, EpochManagerError> {
        let vn = self
            .get_validator_node_by_address(epoch, &validator_addr)?
            .ok_or_else(|| EpochManagerError::ValidatorNodeNotRegistered {
                address: validator_addr.to_string(),
                epoch,
            })?;
        self.get_committee_info_for_substate(epoch, vn.shard_key)
    }

    pub(crate) fn get_committee_vns_from_shard_key(
        &self,
        epoch: Epoch,
        substate_address: SubstateAddress,
    ) -> Result<Vec<ValidatorNode<TAddr>>, EpochManagerError> {
        // retrieve the validator nodes for this epoch from database, sorted by shard_key
        let vns = self.get_validator_nodes_per_epoch(epoch)?;
        if vns.is_empty() {
            return Err(EpochManagerError::NoCommitteeVns {
                substate_address,
                epoch,
            });
        }

        let num_committees = calculate_num_committees(vns.len() as u64, self.config.committee_size);
        if num_committees == 1 {
            return Ok(vns);
        }

        // A shard a equal slice of the shard space that a validator fits into
        let shard = substate_address.to_shard(num_committees);

        let mut shards = HashSet::new();
        shards.insert(shard);
        let selected = self.get_committees_for_shards(epoch, shards)?;
        let shard_vns = selected.get(&shard).map(|c| c.members.clone()).unwrap_or_default();

        let mut res = vec![];
        for (_, pub_key) in shard_vns {
            if let Some(vn) = vns.iter().find(|vn| vn.public_key == pub_key) {
                res.push(vn.clone());
            }
        }
        Ok(res)
    }

    pub(crate) fn get_committee_for_substate(
        &self,
        epoch: Epoch,
        substate_address: SubstateAddress,
    ) -> Result<Committee<TAddr>, EpochManagerError> {
        let result = self.get_committee_vns_from_shard_key(epoch, substate_address)?;
        Ok(Committee::new(
            result.into_iter().map(|v| (v.address, v.public_key)).collect(),
        ))
    }

    pub fn get_number_of_committees(&self, epoch: Epoch) -> Result<u32, EpochManagerError> {
        let num_vns = self.get_total_validator_count(epoch)?;
        Ok(calculate_num_committees(num_vns, self.config.committee_size))
    }

    pub fn get_validator_nodes_per_epoch(&self, epoch: Epoch) -> Result<Vec<ValidatorNode<TAddr>>, EpochManagerError> {
        let mut tx = self.global_db.create_transaction()?;
        let db_vns = self
            .global_db
            .validator_nodes(&mut tx)
            .get_all_within_epoch(epoch, self.config.validator_node_sidechain_id.as_ref())?;
        let vns = db_vns.into_iter().map(Into::into).collect();
        Ok(vns)
    }

    pub async fn on_scanning_complete(&mut self) -> Result<(), EpochManagerError> {
        self.refresh_base_layer_consensus_constants().await?;

        if !self.is_initial_base_layer_sync_complete {
            info!(
                target: LOG_TARGET,
                "🌟 Initial base layer sync complete. Current epoch is {}", self.current_epoch
            );
            self.publish_event(EpochManagerEvent::EpochChanged(self.current_epoch));
            self.is_initial_base_layer_sync_complete = true;
            for reply in self.waiting_for_scanning_complete.drain(..) {
                let _ignore = reply.send(Ok(()));
            }
        }

        Ok(())
    }

    pub fn add_notify_on_scanning_complete(&mut self, reply: oneshot::Sender<Result<(), EpochManagerError>>) {
        if self.is_initial_base_layer_sync_complete {
            let _ignore = reply.send(Ok(()));
        } else {
            self.waiting_for_scanning_complete.push(reply);
        }
    }

    pub async fn remaining_registration_epochs(&mut self) -> Result<Option<Epoch>, EpochManagerError> {
        let last_registration_epoch = match self.last_registration_epoch()? {
            Some(epoch) => epoch,
            None => return Ok(None),
        };

        let constants = self.get_base_layer_consensus_constants().await?;
        let expiry = constants.validator_node_registration_expiry();

        // Note this can be negative in some cases
        let num_blocks_since_last_reg = self.current_epoch.saturating_sub(last_registration_epoch);

        // None indicates that we are not registered, or a previous registration has expired
        Ok(expiry.checked_sub(num_blocks_since_last_reg))
    }

    pub fn get_our_validator_node(&self, epoch: Epoch) -> Result<ValidatorNode<TAddr>, EpochManagerError> {
        let vn = self
            .get_validator_node_by_public_key(epoch, &self.node_public_key)?
            .ok_or_else(|| EpochManagerError::ValidatorNodeNotRegistered {
                address: TAddr::try_from_public_key(&self.node_public_key)
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| self.node_public_key.to_string()),
                epoch,
            })?;
        Ok(vn)
    }

    pub fn get_total_validator_count(&self, epoch: Epoch) -> Result<u64, EpochManagerError> {
        self.get_validator_nodes_per_epoch(epoch)?
            .len()
            .try_into()
            .map_err(|_| EpochManagerError::IntegerOverflow {
                func: "get_total_validator_count",
            })
    }

    pub fn get_num_committees(&self, epoch: Epoch) -> Result<u32, EpochManagerError> {
        let total_vns = self.get_total_validator_count(epoch)?;
        let committee_size = self.config.committee_size;
        let num_committees = calculate_num_committees(total_vns, committee_size);
        Ok(num_committees)
    }

    pub fn get_committee_info_for_substate(
        &self,
        epoch: Epoch,
        substate_address: SubstateAddress,
    ) -> Result<CommitteeInfo, EpochManagerError> {
        let num_committees = self.get_number_of_committees(epoch)?;
        let shard = substate_address.to_shard(num_committees);
        let mut tx = self.global_db.create_transaction()?;
        let mut validator_node_db = self.global_db.validator_nodes(&mut tx);
        let num_validators =
            validator_node_db.count_in_bucket(epoch, self.config.validator_node_sidechain_id.as_ref(), shard)?;
        let num_validators = u32::try_from(num_validators).map_err(|_| EpochManagerError::IntegerOverflow {
            func: "get_committee_shard",
        })?;
        Ok(CommitteeInfo::new(num_committees, num_validators, shard))
    }

    pub fn get_local_committee_info(&self, epoch: Epoch) -> Result<CommitteeInfo, EpochManagerError> {
        let vn = self
            .get_validator_node_by_public_key(epoch, &self.node_public_key)?
            .ok_or_else(|| EpochManagerError::ValidatorNodeNotRegistered {
                address: self.node_public_key.to_string(),
                epoch,
            })?;
        self.get_committee_info_for_substate(epoch, vn.shard_key)
    }

    pub(crate) fn get_committees_for_shards(
        &self,
        epoch: Epoch,
        shards: HashSet<Shard>,
    ) -> Result<HashMap<Shard, Committee<TAddr>>, EpochManagerError> {
        let mut tx = self.global_db.create_transaction()?;
        let mut validator_node_db = self.global_db.validator_nodes(&mut tx);
        let committees = validator_node_db.get_committees_for_shards(epoch, shards)?;
        Ok(committees)
    }

    pub fn get_fee_claim_public_key(&self) -> Result<Option<PublicKey>, EpochManagerError> {
        let mut tx = self.global_db.create_transaction()?;
        let mut metadata = self.global_db.metadata(&mut tx);
        let fee_claim_public_key = metadata.get_metadata(MetadataKey::EpochManagerFeeClaimPublicKey)?;
        Ok(fee_claim_public_key)
    }

    pub fn set_fee_claim_public_key(&mut self, public_key: PublicKey) -> Result<(), EpochManagerError> {
        let mut tx = self.global_db.create_transaction()?;
        let mut metadata = self.global_db.metadata(&mut tx);
        metadata.set_metadata(MetadataKey::EpochManagerFeeClaimPublicKey, &public_key)?;
        tx.commit()?;
        Ok(())
    }

    fn publish_event(&mut self, event: EpochManagerEvent) {
        let _ignore = self.tx_events.send(event);
    }

    pub async fn get_base_layer_block_height(&self, hash: FixedHash) -> Result<Option<u64>, EpochManagerError> {
        let mut tx = self.global_db.create_transaction()?;
        let mut base_layer_hashes = self.global_db.base_layer_hashes(&mut tx);
        let info = base_layer_hashes
            .get_base_layer_block_height(hash)?
            .map(|info| info.height);
        Ok(info)
    }
}

fn calculate_num_committees(num_vns: u64, committee_size: NonZeroU32) -> u32 {
    // Number of committees is proportional to the number of validators available.
    // We cap the number of committees to u32::MAX (for a committee_size of 10 that's over 42 billion validators)
    cmp::min(
        cmp::max(1, num_vns / u64::from(committee_size.get())),
        u64::from(u32::MAX),
    ) as u32
}
