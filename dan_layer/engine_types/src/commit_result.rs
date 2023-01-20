//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use serde::{Deserialize, Serialize};
use tari_template_lib::Hash;

use crate::{execution_result::ExecutionResult, logs::LogEntry, substate::SubstateDiff};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalizeResult {
    pub transaction_hash: Hash,
    pub logs: Vec<LogEntry>,
    pub execution_results: Vec<ExecutionResult>,
    pub result: TransactionResult,
}

impl FinalizeResult {
    pub fn new(transaction_hash: Hash, logs: Vec<LogEntry>, result: TransactionResult) -> Self {
        Self {
            transaction_hash,
            logs,
            execution_results: Vec::new(),
            result,
        }
    }

    pub fn reject(transaction_hash: Hash, reason: RejectReason) -> Self {
        Self::new(transaction_hash, Vec::new(), TransactionResult::Reject(reason))
    }

    pub fn is_accept(&self) -> bool {
        matches!(self.result, TransactionResult::Accept(_))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransactionResult {
    Accept(SubstateDiff),
    Reject(RejectReason),
}

impl TransactionResult {
    pub fn is_accept(&self) -> bool {
        matches!(self, Self::Accept(_))
    }

    pub fn accept(&self) -> Option<&SubstateDiff> {
        match self {
            Self::Accept(substate_diff) => Some(substate_diff),
            Self::Reject(_) => None,
        }
    }

    pub fn reject(&self) -> Option<&RejectReason> {
        match self {
            Self::Accept(_) => None,
            Self::Reject(reject_result) => Some(reject_result),
        }
    }

    pub fn expect(self, msg: &str) -> SubstateDiff {
        match self {
            Self::Accept(substate_diff) => substate_diff,
            Self::Reject(reject_result) => {
                panic!("{}: {:?}", msg, reject_result);
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RejectReason {
    ShardsNotPledged(String),
    ExecutionFailure(String),
    PreviousQcRejection,
    ShardPledgedToAnotherPayload(String),
    ShardRejected(String),
}

impl std::fmt::Display for RejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            RejectReason::ShardsNotPledged(msg) => write!(f, "Shards not pledged: {}", msg),
            RejectReason::ExecutionFailure(msg) => write!(f, "Execution failure: {}", msg),
            RejectReason::PreviousQcRejection => write!(f, "Previous QC was a rejection"),
            RejectReason::ShardPledgedToAnotherPayload(msg) => write!(f, "Shard pledged to another payload: {}", msg),
            RejectReason::ShardRejected(msg) => write!(f, "Shard was rejected: {}", msg),
        }
    }
}
