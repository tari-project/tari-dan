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

use std::fmt::{self, Display, Formatter};

use serde::{Deserialize, Serialize};
use tari_template_lib::Hash;
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::{
    events::Event,
    fees::FeeReceipt,
    instruction_result::InstructionResult,
    logs::LogEntry,
    serde_with,
    substate::SubstateDiff,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ExecuteResult {
    /// The finalized result to commit. If the fee transaction succeeds but the transaction fails, this will be accept.
    pub finalize: FinalizeResult,
}

impl ExecuteResult {
    pub fn expect_success(&self) -> &SubstateDiff {
        let diff = self.expect_finalization_success();

        if let Some(reason) = self.finalize.full_reject() {
            panic!("Transaction failed: {}", reason);
        }

        diff
    }

    pub fn expect_failure(&self) -> &RejectReason {
        if let Some(reason) = self.finalize.result.full_reject() {
            reason
        } else {
            panic!("Transaction succeeded but it was expected to fail");
        }
    }

    pub fn expect_finalization_failure(&self) -> &RejectReason {
        match self.finalize.result {
            TransactionResult::Accept(_) => panic!("Expected transaction to fail but it succeeded"),
            TransactionResult::AcceptFeeRejectRest(_, ref reason) => reason,
            TransactionResult::Reject(ref reason) => reason,
        }
    }

    pub fn expect_fee_accept_transaction_reject(&self) -> (&SubstateDiff, &RejectReason) {
        match self.finalize.result {
            TransactionResult::AcceptFeeRejectRest(ref diff, ref reason) => (diff, reason),
            _ => panic!(
                "Expected fee accept, transaction reject but got {:?}",
                self.finalize.result
            ),
        }
    }

    pub fn expect_transaction_failure(&self) -> &RejectReason {
        if let Some(reason) = self.finalize.full_reject() {
            reason
        } else {
            panic!("Transaction succeeded but it was expected to fail");
        }
    }

    pub fn expect_finalization_success(&self) -> &SubstateDiff {
        match self.finalize.result {
            TransactionResult::Accept(ref diff) => diff,
            TransactionResult::AcceptFeeRejectRest(ref diff, _) => diff,
            TransactionResult::Reject(ref reason) => panic!("Transaction failed: {}", reason),
        }
    }

    pub fn expect_fees_paid_in_full(&self) -> &FeeReceipt {
        let receipt = &self.finalize.fee_receipt;
        assert!(receipt.is_paid_in_full(), "Fees not paid in full");
        receipt
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct FinalizeResult {
    #[serde(with = "serde_with::hex")]
    #[cfg_attr(feature = "ts", ts(type = "Uint8Array"))]
    pub transaction_hash: Hash,
    pub events: Vec<Event>,
    pub logs: Vec<LogEntry>,
    pub execution_results: Vec<InstructionResult>,
    pub result: TransactionResult,
    pub fee_receipt: FeeReceipt,
}

impl FinalizeResult {
    pub fn new(
        transaction_hash: Hash,
        logs: Vec<LogEntry>,
        events: Vec<Event>,
        result: TransactionResult,
        fee_receipt: FeeReceipt,
    ) -> Self {
        Self {
            transaction_hash,
            logs,
            events,
            execution_results: Vec::new(),
            result,
            fee_receipt,
        }
    }

    pub fn new_rejected(transaction_hash: Hash, reason: RejectReason) -> Self {
        Self {
            transaction_hash,
            logs: vec![],
            events: vec![],
            execution_results: Vec::new(),
            result: TransactionResult::Reject(reason),
            fee_receipt: FeeReceipt::default(),
        }
    }

    /// Returns the accept diff if the transaction was accepted, otherwise None.
    /// Acceptance includes fee-only acceptance.
    pub fn accept(&self) -> Option<&SubstateDiff> {
        self.result.accept()
    }

    pub fn into_accept(self) -> Option<SubstateDiff> {
        match self.result {
            TransactionResult::Accept(diff) => Some(diff),
            TransactionResult::AcceptFeeRejectRest(diff, _) => Some(diff),
            TransactionResult::Reject(_) => None,
        }
    }

    pub fn reject(&self) -> Option<&RejectReason> {
        self.result.reject()
    }

    pub fn full_reject(&self) -> Option<&RejectReason> {
        self.result.full_reject()
    }

    pub fn fee_accept_transaction_reject(&self) -> Option<(&SubstateDiff, &RejectReason)> {
        self.result.fee_accept_transaction_reject()
    }

    pub fn is_full_accept(&self) -> bool {
        matches!(self.result, TransactionResult::Accept(_))
    }

    pub fn is_accept(&self) -> bool {
        self.is_full_accept() || self.is_fee_only()
    }

    pub fn is_fee_only(&self) -> bool {
        matches!(self.result, TransactionResult::AcceptFeeRejectRest(_, _))
    }

    pub fn is_reject(&self) -> bool {
        matches!(self.result, TransactionResult::Reject(_))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub enum TransactionResult {
    Accept(SubstateDiff),
    AcceptFeeRejectRest(SubstateDiff, RejectReason),
    Reject(RejectReason),
}

impl TransactionResult {
    pub fn is_accept(&self) -> bool {
        matches!(self, Self::Accept(_) | Self::AcceptFeeRejectRest(_, _))
    }

    pub fn is_reject(&self) -> bool {
        matches!(self, Self::Reject(_))
    }

    pub fn accept(&self) -> Option<&SubstateDiff> {
        match self {
            Self::Accept(substate_diff) => Some(substate_diff),
            Self::AcceptFeeRejectRest(substate_diff, _) => Some(substate_diff),
            Self::Reject(_) => None,
        }
    }

    pub fn reject(&self) -> Option<&RejectReason> {
        match self {
            Self::Accept(_) => None,
            Self::AcceptFeeRejectRest(_, _) => None,
            Self::Reject(reject_result) => Some(reject_result),
        }
    }

    pub fn fee_accept_transaction_reject(&self) -> Option<(&SubstateDiff, &RejectReason)> {
        match self {
            Self::AcceptFeeRejectRest(substate_diff, reject_result) => Some((substate_diff, reject_result)),
            _ => None,
        }
    }

    pub fn full_reject(&self) -> Option<&RejectReason> {
        match self {
            Self::Accept(_) => None,
            Self::AcceptFeeRejectRest(_, reject_result) => Some(reject_result),
            Self::Reject(reject_result) => Some(reject_result),
        }
    }

    pub fn expect(self, msg: &str) -> SubstateDiff {
        match self {
            Self::Accept(substate_diff) => substate_diff,
            Self::AcceptFeeRejectRest(substate_diff, _) => substate_diff,
            Self::Reject(reject_result) => {
                panic!("{}: {:?}", msg, reject_result);
            },
        }
    }
}

impl Display for TransactionResult {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Accept(diff) => write!(f, "Accept({} up, {} down)", diff.up_len(), diff.down_len()),
            Self::AcceptFeeRejectRest(diff, reason) => write!(
                f,
                "Accept({} up, {} down), Reject {}",
                diff.up_len(),
                diff.down_len(),
                reason
            ),
            Self::Reject(reason) => write!(f, "Reject: {}", reason),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub enum RejectReason {
    ShardsNotPledged(String),
    ExecutionFailure(String),
    PreviousQcRejection,
    ShardPledgedToAnotherPayload(String),
    ShardRejected(String),
    FeeTransactionFailed,
    FeesNotPaid(String),
}

impl std::fmt::Display for RejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            RejectReason::ShardsNotPledged(msg) => write!(f, "Shards not pledged: {}", msg),
            RejectReason::ExecutionFailure(msg) => write!(f, "Execution failure: {}", msg),
            RejectReason::PreviousQcRejection => write!(f, "Previous QC was a rejection"),
            RejectReason::ShardPledgedToAnotherPayload(msg) => write!(f, "Shard pledged to another payload: {}", msg),
            RejectReason::ShardRejected(msg) => write!(f, "Shard was rejected: {}", msg),
            RejectReason::FeeTransactionFailed => write!(f, "Fee transaction failed"),
            RejectReason::FeesNotPaid(msg) => write!(f, "Fee not paid: {}", msg),
        }
    }
}
