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

use minicbor::{CborLen, Decode, Encode};
use serde::{Deserialize, Serialize};
use tari_consensus_types::{BlockId, Decision};
use tari_ootle_common_types::{Epoch, NodeHeight};
use tari_ootle_storage::consensus_models::{
    Evidence,
    LeaderFee,
    LockedEpoch,
    TransactionPoolRecord,
    TransactionPoolStage,
};
use tari_ootle_transaction::TransactionId;

use crate::{
    codecs::{BlockIdCodec, BytesCodec, DefaultCodec, EpochCodec, KeyPrefix, NumberCodec, TransactionIdCodec},
    column_families::cf_names,
    prefixed,
    traits::{Cf, QueryCf},
};

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, CborLen)]
pub struct TransactionPoolStateUpdateData {
    #[n(0)]
    pub block_id: BlockId,
    #[n(1)]
    pub block_height: NodeHeight,
    #[n(2)]
    pub transaction_id: TransactionId,
    #[n(3)]
    pub evidence: Evidence,
    #[n(4)]
    pub transaction_fee: u64,
    #[n(5)]
    pub leader_fee: Option<LeaderFee>,
    #[n(6)]
    pub stage: TransactionPoolStage,
    #[n(7)]
    pub local_decision: Decision,
    #[n(8)]
    pub remote_decision: Option<Decision>,
    #[n(9)]
    pub locked_epoch: Option<LockedEpoch>,
    #[n(10)]
    pub is_ready: bool,
    #[n(11)]
    pub exhaust_burn: u64,
}

impl TransactionPoolStateUpdateData {
    pub fn merge_into(self, other: &mut TransactionPoolRecord) {
        other.set_evidence(self.evidence);
        other.set_transaction_fee(self.transaction_fee);
        other.set_exhaust_burn(self.exhaust_burn);
        if let Some(leader_fee) = self.leader_fee {
            other.set_leader_fee(leader_fee);
        }
        other.set_pending_stage(Some(self.stage));
        other.set_local_decision(self.local_decision);
        if let Some(remote_decision) = self.remote_decision {
            other.set_remote_decision(remote_decision);
        }
        other.set_locked_epoch(self.locked_epoch);
        other.set_is_ready(self.is_ready);
    }
}

prefixed!(TransactionPoolStateUpdatePrefix, KeyPrefix::TransactionPoolStateUpdates);

pub struct TransactionPoolStateUpdateCf;

impl Cf for TransactionPoolStateUpdateCf {
    type Key = (BlockId, TransactionId);
    type KeyCodec = (BlockIdCodec, TransactionIdCodec);
    type Prefix = TransactionPoolStateUpdatePrefix;
    type Value = TransactionPoolStateUpdateData;
    type ValueCodec = DefaultCodec<TransactionPoolStateUpdateData>;

    fn name() -> &'static str {
        cf_names::TRANSACTIONS
    }
}

pub struct ByBlockIdQuery;

impl QueryCf for ByBlockIdQuery {
    type Cf = TransactionPoolStateUpdateCf;
    type Key = BlockId;
    type KeyCodec = BlockIdCodec;
}

prefixed!(
    TransactionPoolStateUpdateDebugHistoryPrefix,
    KeyPrefix::TransactionPoolStateUpdateDebugHistory
);

pub struct TransactionPoolStateUpdateDebugHistoryCf;

impl Cf for TransactionPoolStateUpdateDebugHistoryCf {
    type Key = (Epoch, NodeHeight, TransactionId);
    type KeyCodec = (EpochCodec, NumberCodec<NodeHeight>, BytesCodec);
    type Prefix = TransactionPoolStateUpdateDebugHistoryPrefix;
    type Value = TransactionPoolStateUpdateData;
    type ValueCodec = DefaultCodec<TransactionPoolStateUpdateData>;

    fn name() -> &'static str {
        cf_names::DIAGNOSTICS
    }
}
