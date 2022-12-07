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

use std::{collections::HashMap, convert::TryFrom};

use futures::StreamExt;
use log::info;
use tari_comms::protocol::rpc::RpcStatus;
use tari_dan_common_types::{Epoch, NodeHeight, ObjectPledge, PayloadId, ShardId, SubstateState};
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
        shard_store::{ShardStore, ShardStoreTransaction},
        StorageError,
    },
};
use tari_dan_engine::transaction::Transaction;
use tari_dan_storage_sqlite::sqlite_shard_store_factory::SqliteShardStore;
use tari_engine_types::commit_result::FinalizeResult;
use thiserror::Error;

use crate::{
    p2p::{
        proto::rpc::VnStateSyncResponse,
        services::{
            epoch_manager::handle::EpochManagerHandle,
            rpc_client::TariCommsValidatorNodeClientFactory,
            template_manager::TemplateManager,
        },
    },
    payload_processor::TariDanPayloadProcessor,
};

const LOG_TARGET: &str = "tari::validator_node::dry_run_trasaction_processor";

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
}

#[derive(Clone)]
pub struct DryRunTransactionProcessor {
    epoch_manager: EpochManagerHandle,
    payload_processor: TariDanPayloadProcessor<TemplateManager>,
    shard_store: SqliteShardStore,
    validator_node_client_factory: TariCommsValidatorNodeClientFactory,
}

impl DryRunTransactionProcessor {
    pub fn new(
        epoch_manager: EpochManagerHandle,
        payload_processor: TariDanPayloadProcessor<TemplateManager>,
        shard_store: SqliteShardStore,
        validator_node_client_factory: TariCommsValidatorNodeClientFactory,
    ) -> Self {
        Self {
            epoch_manager,
            payload_processor,
            shard_store,
            validator_node_client_factory,
        }
    }

    pub async fn process_transaction(
        &self,
        transaction: Transaction,
    ) -> Result<FinalizeResult, DryRunTransactionProcessorError> {
        // get the list of involved shards for the transaction
        let payload = TariDanPayload::new(transaction.clone());
        let involved_shards = payload.involved_shards();

        // get the pledges for all local shards
        let mut shard_pledges = self.get_local_pledges(involved_shards.clone()).await?;

        // get non local shard pledges
        let epoch = self.epoch_manager.current_epoch().await?;
        let local_involved_shards: Vec<ShardId> = shard_pledges.keys().copied().collect();
        let remote_involved_shards: Vec<ShardId> = involved_shards
            .into_iter()
            .filter(|s| !local_involved_shards.contains(s))
            .collect();
        for shard_id in remote_involved_shards {
            let pledge = self.get_remote_pledge(shard_id, epoch).await?;
            shard_pledges.insert(shard_id, pledge);
        }

        // execute the payload in the WASM engine and return the result
        let result = self.payload_processor.process_payload(payload, shard_pledges)?;
        Ok(result)
    }

    async fn get_local_pledges(
        &self,
        involved_shards: Vec<ShardId>,
    ) -> Result<HashMap<ShardId, Option<ObjectPledge>>, DryRunTransactionProcessorError> {
        let tx = self.shard_store.create_tx().unwrap();
        let inventory = tx.get_state_inventory().unwrap();

        let local_shard_ids: Vec<ShardId> = involved_shards.into_iter().filter(|s| inventory.contains(s)).collect();
        let mut local_pledges = HashMap::new();
        let local_substates = tx.get_substate_states(&local_shard_ids)?;
        for substate in local_substates {
            let local_pledge = ObjectPledge {
                shard_id: substate.shard(),
                current_state: substate.substate().clone(),
                pledged_to_payload: substate.payload_id(),
                pledged_until: substate.height(),
            };
            local_pledges.insert(substate.shard(), Some(local_pledge));
        }

        Ok(local_pledges)
    }

    pub async fn get_remote_pledge(
        &self,
        shard_id: ShardId,
        epoch: Epoch,
    ) -> Result<Option<ObjectPledge>, DryRunTransactionProcessorError> {
        let committee = self.epoch_manager.get_committee(epoch, shard_id).await?;

        for vn_public_key in committee.members {
            // build a client with the VN
            let mut sync_vn_client = self.validator_node_client_factory.create_client(&vn_public_key);
            let mut sync_vn_rpc_client = sync_vn_client.create_connection().await?;

            // request the shard substate to the VN
            let shard_id_proto: crate::p2p::proto::common::ShardId = shard_id.into();
            let request = crate::p2p::proto::rpc::VnStateSyncRequest {
                start_shard_id: Some(shard_id_proto.clone()),
                end_shard_id: Some(shard_id_proto),
                inventory: vec![],
            };

            // get the VN's response
            let mut vn_state_stream = match sync_vn_rpc_client.vn_state_sync(request).await {
                Ok(stream) => stream,
                Err(e) => {
                    info!(target: LOG_TARGET, "Unable to connect to peer: {} ", e.to_string(),);
                    // we do not stop when an indiviual VN does not respond, we try all VNs
                    continue;
                },
            };

            // extract the shard pledge from the response
            if let Some(resp) = vn_state_stream.next().await {
                match Self::extract_pledge_from_vn_sync_response(resp) {
                    Ok(pledge) => return Ok(Some(pledge)),
                    Err(error_msg) => {
                        info!(target: LOG_TARGET, "Unable to extract pledge from peer: {} ", error_msg,);
                        // we do not stop when an indiviual VN does not respond correctly, we try all Vns
                        continue;
                    },
                };
            }
        }

        // The shard was not found in any VN
        Ok(None)
    }

    fn extract_pledge_from_vn_sync_response(
        resp: Result<VnStateSyncResponse, RpcStatus>,
    ) -> Result<ObjectPledge, String> {
        let msg = resp.map_err(|e| e.to_string())?;

        let shard_id = ShardId::try_from(msg.shard_id.ok_or("Unexpected response")?).map_err(|e| e.to_string())?;
        let current_state =
            SubstateState::try_from(msg.substate_state.ok_or("Unexpected response")?).map_err(|e| e.to_string())?;
        let pledged_to_payload = PayloadId::try_from(msg.payload_id).map_err(|e| e.to_string())?;
        let pledged_until = NodeHeight::from(msg.node_height);

        let pledge = ObjectPledge {
            shard_id,
            current_state,
            pledged_to_payload,
            pledged_until,
        };

        Ok(pledge)
    }
}
