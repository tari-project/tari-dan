//  Copyright 2021, The Tari Project
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

mod rpc_impl;
mod state_sync_task;

/// The status returned to a peer whose request needs consensus in a state this node is not in.
pub(crate) const CONSENSUS_NOT_RUNNING: &str = "Consensus is not running on this node";

pub use rpc_impl::ValidatorNodeRpcServiceImpl;
use tari_epoch_manager::service::EpochManagerHandle;
use tari_ootle_p2p::PeerAddress;
use tari_ootle_storage::StateStore;
use tari_validator_node_rpc::rpc_service::ValidatorNodeRpcServer;

use crate::{consensus::ConsensusHandle, p2p::services::mempool::MempoolHandle};

pub fn create_tari_validator_node_rpc_service<TStateStore: StateStore + Send + Sync + Clone + 'static>(
    epoch_manager: EpochManagerHandle<PeerAddress>,
    shard_store_store: TStateStore,
    mempool: MempoolHandle,
    consensus: ConsensusHandle,
    max_transaction_size_bytes: usize,
    epoch_history_length: u64,
) -> ValidatorNodeRpcServer<ValidatorNodeRpcServiceImpl<TStateStore>> {
    ValidatorNodeRpcServer::new(ValidatorNodeRpcServiceImpl::new(
        epoch_manager,
        shard_store_store,
        mempool,
        consensus,
        max_transaction_size_bytes,
        epoch_history_length,
    ))
}
