//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt,
    fmt::{Display, Formatter},
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use strum::ParseError;
use strum_macros::{AsRefStr, EnumString};
use tari_engine_types::commit_result::{RejectReason, TransactionResult};
#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Debug, Clone, thiserror::Error)]
pub enum FromStrConversionError {
    #[error("Invalid Decision string '{0}'")]
    InvalidDecision(String),
    #[error("Invalid Abort reason string '{0}': {1}")]
    InvalidAbortReason(String, ParseError),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub enum Decision {
    /// Decision to COMMIT the transaction
    Commit,
    /// Decision to ABORT the transaction
    Abort(AbortReason),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize, AsRefStr, EnumString)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub enum AbortReason {
    None,
    TransactionAtomMustBeAbort,
    TransactionAtomMustBeCommit,
    InputLockConflict,
    LockInputsFailed,
    LockOutputsFailed,
    LockInputsOutputsFailed,
    LeaderProposalVsLocalDecisionMismatch,
    InvalidTransaction,
    ExecutionFailure,
    OneOrMoreInputsNotFound,
    ForeignShardGroupDecidedToAbort,
    FeesNotPaid,
    EarlyAbort,
}

impl From<&RejectReason> for AbortReason {
    fn from(reject_reason: &RejectReason) -> Self {
        match reject_reason {
            RejectReason::Unknown => Self::None,
            RejectReason::InvalidTransaction(_) => Self::InvalidTransaction,
            RejectReason::ExecutionFailure(_) => Self::ExecutionFailure,
            RejectReason::OneOrMoreInputsNotFound(_) => Self::OneOrMoreInputsNotFound,
            RejectReason::FailedToLockInputs(_) => Self::LockInputsFailed,
            RejectReason::FailedToLockOutputs(_) => Self::LockOutputsFailed,
            RejectReason::ForeignShardGroupDecidedToAbort { .. } => Self::ForeignShardGroupDecidedToAbort,
            RejectReason::FeesNotPaid(_) => Self::FeesNotPaid,
        }
    }
}

impl Decision {
    pub fn is_commit(&self) -> bool {
        matches!(self, Decision::Commit)
    }

    pub fn is_abort(&self) -> bool {
        matches!(self, Decision::Abort(_))
    }

    pub fn and(self, other: Self) -> Self {
        match self {
            Decision::Commit => other,
            Decision::Abort(reason) => Decision::Abort(reason),
        }
    }

    pub fn as_string(&self) -> String {
        match self {
            Decision::Commit => String::from("Commit"),
            Decision::Abort(reason) => format!("Abort({})", reason.as_ref()),
        }
    }
}

impl Display for Decision {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_string().as_str())
    }
}

impl FromStr for Decision {
    type Err = FromStrConversionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Commit" => Ok(Decision::Commit),
            "Abort" => {
                // to stay compatible with previous messages
                Ok(Decision::Abort(AbortReason::None))
            },
            _ => {
                // abort with reason
                if s.starts_with("Abort(") {
                    let mut reason = s.replace("Abort(", "");
                    reason.pop(); // remove last char ')'
                    return Ok(Decision::Abort(AbortReason::from_str(reason.as_str()).map_err(
                        |error| FromStrConversionError::InvalidAbortReason(s.to_string(), error),
                    )?));
                }

                Err(FromStrConversionError::InvalidDecision(s.to_string()))
            },
        }
    }
}

impl From<&TransactionResult> for Decision {
    fn from(result: &TransactionResult) -> Self {
        if result.is_accept() {
            Decision::Commit
        } else if let TransactionResult::Reject(reject_reason) = result {
            Decision::Abort(AbortReason::from(reject_reason))
        } else {
            Decision::Abort(AbortReason::None)
        }
    }
}
