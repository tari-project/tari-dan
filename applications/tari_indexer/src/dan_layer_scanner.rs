//  Copyright 2023, The Tari Project
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

use futures::StreamExt;
use log::*;
use serde::{Deserialize, Serialize};
use tari_crypto::ristretto::RistrettoPublicKey;
use tari_dan_app_grpc::proto::rpc::VnStateSyncResponse;
use tari_dan_app_utilities::epoch_manager::EpochManagerHandle;
use tari_dan_common_types::{Epoch, ShardId};
use tari_dan_core::services::{epoch_manager::EpochManager, ValidatorNodeClientFactory};
use tari_engine_types::substate::{Substate, SubstateAddress};

use crate::p2p::services::rpc_client::TariCommsValidatorNodeClientFactory;

const LOG_TARGET: &str = "tari::indexer::dan_layer_scanner";

pub struct DanLayerScanner {
    epoch_manager: EpochManagerHandle,
    validator_node_client_factory: TariCommsValidatorNodeClientFactory,
}

impl DanLayerScanner {
    pub fn new(
        epoch_manager: EpochManagerHandle,
        validator_node_client_factory: TariCommsValidatorNodeClientFactory,
    ) -> Self {
        Self {
            epoch_manager,
            validator_node_client_factory,
        }
    }

    pub async fn get_substate(&self, substate_address: &SubstateAddress, version: Option<u32>) -> Option<Substate> {
        info!(target: LOG_TARGET, "get_substate: {} ", substate_address);

        let epoch = match self.epoch_manager.current_epoch().await {
            Ok(epoch) => epoch,
            Err(e) => {
                error!(target: LOG_TARGET, "Could not retrieve the current epoch: {} ", e);
                return None;
            },
        };

        let result = match version {
            Some(version) => {
                self.get_specific_substate_from_commitee(substate_address, version, epoch)
                    .await
            },
            None => self.get_latest_substate_from_commitee(substate_address, epoch).await,
        };

        match result {
            SubstateResult::Up(substate) => Some(substate),
            SubstateResult::Down(substate) => Some(substate),
            SubstateResult::DoesNotExist => None,
        }
    }

    async fn get_latest_substate_from_commitee(
        &self,
        substate_address: &SubstateAddress,
        epoch: Epoch,
    ) -> SubstateResult {
        // we keep asking from version 0 upwards
        let mut version = 0;
        loop {
            let substate_result = self
                .get_specific_substate_from_commitee(substate_address, version, epoch)
                .await;
            match substate_result {
                // when it's a "Down" state, we need to ask a higher version until we find an "Up" or "DoesNotExist"
                SubstateResult::Down(_) => {
                    version += 1;
                },
                _ => return substate_result,
            }
        }
    }

    async fn get_specific_substate_from_commitee(
        &self,
        substate_address: &SubstateAddress,
        version: u32,
        epoch: Epoch,
    ) -> SubstateResult {
        let shard_id = ShardId::from_address(substate_address, version);
        let committee = match self.epoch_manager.get_committee(epoch, shard_id).await {
            Ok(committee) => committee,
            Err(e) => {
                error!(
                    target: LOG_TARGET,
                    "Could not get commitee for substate {}:{} on epoch {}: {}", substate_address, version, epoch, e
                );
                return SubstateResult::DoesNotExist;
            },
        };

        for vn_public_key in &committee.members {
            match self.get_substate_from_vn(vn_public_key, shard_id).await {
                Ok(substate_result) => return substate_result,
                Err(e) => {
                    // We ignore a single VN error and keep querying the rest of the committee
                    error!(
                        target: LOG_TARGET,
                        "Could not get substate {}:{} from vn {} on epoch {}: {}",
                        substate_address,
                        version,
                        vn_public_key,
                        epoch,
                        e
                    );
                },
            };
            let res = self.get_substate_from_vn(vn_public_key, shard_id).await;

            if let Ok(substate_result) = res {
                return substate_result;
            }
        }

        error!(
            target: LOG_TARGET,
            "Could not get substate {}:{} from any of the validator nodes on epoch {}",
            substate_address,
            version,
            epoch
        );

        SubstateResult::DoesNotExist
    }

    async fn get_substate_from_vn(
        &self,
        vn_public_key: &RistrettoPublicKey,
        shard_id: ShardId,
    ) -> Result<SubstateResult, anyhow::Error> {
        // build a client with the VN
        let mut sync_vn_client = self.validator_node_client_factory.create_client(vn_public_key);
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
                return Err(e.into());
            },
        };

        // return the substate from the response (there is going to be only 0 or 1 result)
        if let Some(msg) = vn_state_stream.next().await {
            let msg = msg?;
            let state = extract_state_from_vn_response(msg)?;
            return Ok(state);
        }

        Ok(SubstateResult::DoesNotExist)
    }
}

fn extract_state_from_vn_response(msg: VnStateSyncResponse) -> Result<SubstateResult, anyhow::Error> {
    let substate = Substate::from_bytes(&msg.substate)?;

    let result = if msg.destroyed_payload_id.is_empty() {
        SubstateResult::Up(substate)
    } else {
        SubstateResult::Down(substate)
    };

    Ok(result)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum SubstateResult {
    DoesNotExist,
    Up(Substate),
    Down(Substate),
}
