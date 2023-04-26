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
    collections::HashMap,
    convert::{TryFrom, TryInto},
    sync::Arc,
};

use futures::StreamExt;
use log::info;
use tari_comms::{protocol::rpc::RpcStatus, NodeIdentity};
use tari_dan_app_grpc::proto::rpc::VnStateSyncResponse;
use tari_dan_app_utilities::epoch_manager::EpochManagerHandle;
use tari_dan_common_types::{Epoch, ObjectPledge, PayloadId, ShardId, SubstateState};
use tari_dan_core::{
    models::{Payload, TariDanPayload},
    services::{
        epoch_manager::{EpochManager, EpochManagerError},
        PayloadProcessor,
        PayloadProcessorError,
        ValidatorNodeClientError,
        ValidatorNodeClientFactory,
    },
    storage::{
        shard_store::{ShardStore, ShardStoreReadTransaction},
        StorageError,
    },
};
use tari_dan_engine::runtime::ConsensusContext;
use tari_dan_storage_sqlite::sqlite_shard_store_factory::SqliteShardStore;
use tari_engine_types::{
    commit_result::{ExecuteResult, RejectReason, TransactionResult},
    substate::{Substate, SubstateAddress},
};
use tari_transaction::{SubstateChange, Transaction};
use thiserror::Error;

use crate::{
    p2p::services::{rpc_client::TariCommsValidatorNodeClientFactory, template_manager::TemplateManager},
    payload_processor::TariDanPayloadProcessor,
};

const LOG_TARGET: &str = "tari::validator_node::dry_run_transaction_processor";

#[derive(Error, Debug)]
pub enum DryRunTransactionProcessorError {
    #[error("PayloadProcessor error: {0}")]
    PayloadProcessor(#[from] PayloadProcessorError),
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),
    #[error("EpochManager error: {0}")]
    EpochManager(#[from] EpochManagerError),
    #[error("Validator node client error: {0}")]
    ValidatorNodeClient(#[from] ValidatorNodeClientError),
    #[error("Rpc error: {0}")]
    RpcRequestFailed(#[from] RpcStatus),
    #[error("Transaction rejected: {reason}")]
    TransactionRejected { reason: RejectReason },
}

#[derive(Clone, Debug)]
pub struct DryRunTransactionProcessor {
    epoch_manager: EpochManagerHandle,
    payload_processor: TariDanPayloadProcessor<TemplateManager>,
    shard_store: SqliteShardStore,
    validator_node_client_factory: TariCommsValidatorNodeClientFactory,
    node_identity: Arc<NodeIdentity>,
}

impl DryRunTransactionProcessor {
    pub fn new(
        epoch_manager: EpochManagerHandle,
        payload_processor: TariDanPayloadProcessor<TemplateManager>,
        shard_store: SqliteShardStore,
        validator_node_client_factory: TariCommsValidatorNodeClientFactory,
        node_identity: Arc<NodeIdentity>,
    ) -> Self {
        Self {
            epoch_manager,
            payload_processor,
            shard_store,
            validator_node_client_factory,
            node_identity,
        }
    }

    pub async fn process_transaction(
        &self,
        transaction: &Transaction,
    ) -> Result<ExecuteResult, DryRunTransactionProcessorError> {
        // get the list of involved shards for the transaction
        let payload = TariDanPayload::new(transaction.clone());
        let involved_shards = payload.involved_shards();

        // get the pledges for all local shards
        let mut shard_pledges = self.get_local_pledges(involved_shards.clone()).await?;

        // get non local shard pledges
        let epoch = self.epoch_manager.current_epoch().await?;
        let missing_involved_shards: Vec<ShardId> = involved_shards
            .into_iter()
            .filter(|s| !shard_pledges.contains_key(s))
            .collect();
        shard_pledges.reserve(missing_involved_shards.len());
        for shard_id in missing_involved_shards {
            let pledge = self.get_remote_pledge(shard_id, payload.to_id(), epoch).await?;
            shard_pledges.insert(shard_id, pledge);
        }

        // execute the payload in the WASM engine and return the result
        let consensus_context = self.get_consensus_context().await?;
        let result = self
            .payload_processor
            .process_payload(payload, shard_pledges, consensus_context)?;

        if let Some(ref fees) = result.fee_receipt {
            info!(target: LOG_TARGET, "Transaction fees: {}", fees.total_fees_charged());
        }

        Ok(result)
    }

    pub async fn calculate_new_outputs(
        &self,
        transaction: &Transaction,
    ) -> Result<Vec<(SubstateAddress, Substate)>, DryRunTransactionProcessorError> {
        let exec_result = self.process_transaction(transaction).await?;
        match exec_result.finalize.result {
            TransactionResult::Accept(diff) => {
                let up_substates = diff.into_up_iter().collect();
                Ok(up_substates)
            },
            TransactionResult::Reject(reason) => Err(DryRunTransactionProcessorError::TransactionRejected { reason }),
        }
    }

    pub async fn calculate_missing_output_shards(
        &self,
        transaction: &Transaction,
    ) -> Result<Vec<ShardId>, DryRunTransactionProcessorError> {
        let known_shard_ids = transaction.meta().involved_shards();
        let missing_shard_ids: Vec<ShardId> = self
            .calculate_new_outputs(transaction)
            .await?
            .iter()
            .map(|(addr, substate)| ShardId::from_address(addr, substate.version()))
            .filter(|s| !known_shard_ids.contains(s))
            .collect();

        Ok(missing_shard_ids)
    }

    // Updateds the transaction to add all missing shards
    pub async fn add_missing_shards(
        &self,
        transaction: &mut Transaction,
    ) -> Result<(), DryRunTransactionProcessorError> {
        // simulate and execution to known the new shard ids that are going to be created by the transaction
        let missing_shard_ids = self.calculate_missing_output_shards(transaction).await?;

        // nothing else to do when the transaction already has everyting it needs
        if missing_shard_ids.is_empty() {
            return Ok(());
        }

        info!(
            target: LOG_TARGET,
            "Adding {} missing shard ids to the transaction",
            missing_shard_ids.len()
        );

        // add all missing shards
        for shard_id in missing_shard_ids {
            transaction
                .meta_mut()
                .involved_objects_mut()
                .insert(shard_id, SubstateChange::Create);
        }

        Ok(())
    }

    async fn get_consensus_context(&self) -> Result<ConsensusContext, DryRunTransactionProcessorError> {
        let current_epoch = self.epoch_manager.current_epoch().await?.as_u64();
        let consensus_context = ConsensusContext { current_epoch };
        Ok(consensus_context)
    }

    async fn get_local_pledges(
        &self,
        involved_shards: Vec<ShardId>,
    ) -> Result<HashMap<ShardId, ObjectPledge>, DryRunTransactionProcessorError> {
        let local_substates = self.shard_store.with_read_tx(|tx| {
            let inventory = tx.get_state_inventory()?;
            let local_shard_ids: Vec<_> = involved_shards.into_iter().filter(|s| inventory.contains(s)).collect();
            tx.get_substate_states(&local_shard_ids)
        })?;

        let mut local_pledges = HashMap::with_capacity(local_substates.len());
        for substate in local_substates {
            let shard_id = substate.shard_id();
            let local_pledge = ObjectPledge {
                shard_id,
                pledged_to_payload: substate.created_payload_id(),
                current_state: substate.into_substate_state(),
            };
            local_pledges.insert(shard_id, local_pledge);
        }

        Ok(local_pledges)
    }

    pub async fn get_remote_pledge(
        &self,
        shard_id: ShardId,
        payload_id: PayloadId,
        epoch: Epoch,
    ) -> Result<ObjectPledge, DryRunTransactionProcessorError> {
        let committee = self.epoch_manager.get_committee(epoch, shard_id).await?;

        for vn_public_key in committee.members {
            if vn_public_key == *self.node_identity.public_key() {
                continue;
            }

            // build a client with the VN
            let mut sync_vn_client = self.validator_node_client_factory.create_client(&vn_public_key);
            let mut sync_vn_rpc_client = sync_vn_client.create_connection().await?;

            // request the shard substate to the VN
            let shard_id_proto: tari_dan_app_grpc::proto::common::ShardId = shard_id.into();
            let request = tari_dan_app_grpc::proto::rpc::VnStateSyncRequest {
                start_shard_id: Some(shard_id_proto.clone()),
                end_shard_id: Some(shard_id_proto),
                inventory: vec![],
            };

            // get the VN's response
            let mut vn_state_stream = match sync_vn_rpc_client.vn_state_sync(request).await {
                Ok(stream) => stream,
                Err(e) => {
                    info!(target: LOG_TARGET, "Unable to connect to peer: {} ", e);
                    // we do not stop when an indiviual VN does not respond, we try all VNs
                    continue;
                },
            };

            // extract the shard pledge from the response
            if let Some(resp) = vn_state_stream.next().await {
                let resp = resp?;
                match Self::extract_pledge_from_vn_sync_response(resp) {
                    Ok(pledge) => return Ok(pledge),
                    Err(error_msg) => {
                        info!(target: LOG_TARGET, "Unable to extract pledge from peer: {} ", error_msg);
                        // we do not stop when an individual VN does not respond correctly, we try all Vns
                        continue;
                    },
                };
            }
        }

        // The shard does not exist on any VN, so we pledge it to be created in this payload
        Ok(ObjectPledge {
            shard_id,
            pledged_to_payload: payload_id,
            current_state: SubstateState::DoesNotExist,
        })
    }

    fn extract_pledge_from_vn_sync_response(msg: VnStateSyncResponse) -> Result<ObjectPledge, anyhow::Error> {
        let shard_id = ShardId::try_from(msg.shard_id)?;
        let pledged_to_payload = PayloadId::try_from(msg.created_payload_id.as_slice())?;

        let current_state = if let Some(deleted_by) = Some(msg.destroyed_payload_id).filter(|p| !p.is_empty()) {
            SubstateState::Down {
                deleted_by: deleted_by.try_into()?,
                fees_accrued: msg.destroyed_fee_accrued,
            }
        } else {
            let substate = Substate::from_bytes(&msg.substate)?;
            SubstateState::Up {
                created_by: msg.created_payload_id.try_into()?,
                address: SubstateAddress::from_bytes(&msg.address)?,
                data: substate,
                fees_accrued: msg.created_fee_accrued,
            }
        };

        let pledge = ObjectPledge {
            shard_id,
            current_state,
            pledged_to_payload,
        };

        Ok(pledge)
    }
}
