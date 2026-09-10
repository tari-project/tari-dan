//  Copyright 2024. The Tari Project
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

pub mod block;
pub mod block_diff;
pub mod block_transaction_execution;
pub mod bookkeeping;
pub mod chain;
pub mod epoch_checkpoint;
pub mod foreign_parked_blocks;
pub mod foreign_proposal;
pub mod foreign_substate_pledge;

pub mod certificates;
pub mod diagnostic_no_vote;
pub mod finalized_transaction;
pub mod lock_conflict;
pub mod missing_transactions;
pub mod parked_block;
pub mod pending_state_tree_diff;
pub mod rollback_history;
pub mod state_transition;
pub mod state_tree;
pub mod state_tree_shard_versions;
pub mod substate;
pub mod substate_locks;
pub mod transaction;
pub mod transaction_pool;
pub mod transaction_pool_state_update;
pub mod validator_node_epoch_stats;

pub(crate) mod cf_names {
    pub(crate) const CHAIN_METADATA: &str = "chain";
    pub(crate) const CERTIFICATES: &str = "certificates";
    pub(crate) const BLOCK: &str = "block";
    pub(crate) const TRANSACTIONS: &str = "transactions";
    pub(crate) const BOOKKEEPING: &str = "bookkeeping";
    pub(crate) const FOREIGN_PROPOSALS: &str = "foreign_proposals";
    pub(crate) const DIAGNOSTICS: &str = "diagnostics";
    pub(crate) const SUBSTATES: &str = "substates";
    pub(crate) const STATE_TREE: &str = "state_tree";
}
