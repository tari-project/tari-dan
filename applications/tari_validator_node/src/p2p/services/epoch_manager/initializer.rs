//  Copyright 2021. The Tari Project
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

use std::sync::Arc;

use tari_comms::NodeIdentity;
use tari_dan_core::consensus_constants::ConsensusConstants;
use tari_dan_storage::global::GlobalDb;
use tari_dan_storage_sqlite::{global::SqliteGlobalDbAdapter, sqlite_shard_store_factory::SqliteShardStore};
use tari_shutdown::ShutdownSignal;
use tokio::sync::mpsc;

use crate::{
    grpc::services::base_node_client::GrpcBaseNodeClient,
    p2p::services::{
        epoch_manager::{epoch_manager_service::EpochManagerService, handle::EpochManagerHandle},
        rpc_client::TariCommsValidatorNodeClientFactory,
    },
};

pub fn spawn(
    global_db: GlobalDb<SqliteGlobalDbAdapter>,
    shard_store: SqliteShardStore,
    base_node_client: GrpcBaseNodeClient,
    consensus_constants: ConsensusConstants,
    shutdown: ShutdownSignal,
    node_identity: Arc<NodeIdentity>,
    validator_node_client_factory: TariCommsValidatorNodeClientFactory,
) -> EpochManagerHandle {
    let (tx_request, rx_request) = mpsc::channel(10);
    let handle = EpochManagerHandle::new(tx_request);
    EpochManagerService::spawn(
        rx_request,
        shutdown,
        global_db,
        shard_store,
        base_node_client,
        consensus_constants,
        node_identity,
        validator_node_client_factory,
    );
    handle
}
