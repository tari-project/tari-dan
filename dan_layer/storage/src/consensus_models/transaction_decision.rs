//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::{
    fmt,
    fmt::{Display, Formatter},
    str::FromStr,
};
use std::slice::Iter;
use tari_engine_types::commit_result::TransactionResult;
#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub enum Decision {
    /// Decision to COMMIT the transaction
    Commit,
    /// Decision to ABORT the transaction
    Abort(AbortReason),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub enum AbortReason {
    TransactionAtomMustBeAbort,
    TransactionAtomMustBeCommit,
    InputLockConflict,
    LockOutputsFailed,
    LockInputsOutputsFailed,
    LeaderProposalVsLocalDecisionMismatch,
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

    pub const fn as_str(&self) -> &'static str {
        match self {
            Decision::Commit => "Commit",
            Decision::Abort(reason) => format!("Abort({:?})", reason).as_str(),
        }
    }
}

// TransactionAtomMustBeAbort,
//     TransactionAtomMustBeCommit,
//     InputLockConflict,
//     LockOutputsFailed,
//     LockInputsOutputsFailed,
//     LeaderProposalVsLocalDecisionMismatch,

impl IntoIterator for AbortReason {
    // TODO
}

impl Display for Decision {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Decision {
    type Err = FromStrConversionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "Commit" {
            Ok(Decision::Commit)
        } else {
            if s.starts_with("Abort(") {
                let mut reason = s.replace("Abort(", "");
                reason.pop(); // remove last char ')'
                return Ok(
                    Decision::Abort(
                        AbortReason::from_str(reason.as_str())?
                    )
                );
            }

            Err(FromStrConversionError::InvalidDecision(s.to_string()))
        }
    }
}

impl FromStr for AbortReason {
    type Err = FromStrConversionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut possible_matches: HashMap<&str, AbortReason> = HashMap::from([
            (format!("{:?}", AbortReason::TransactionAtomMustBeAbort).as_str(), AbortReason::TransactionAtomMustBeAbort),
            (format!("{:?}", AbortReason::TransactionAtomMustBeCommit).as_str(), AbortReason::TransactionAtomMustBeCommit),
            (format!("{:?}", AbortReason::InputLockConflict).as_str(), AbortReason::InputLockConflict),
            (format!("{:?}", AbortReason::LockOutputsFailed).as_str(), AbortReason::LockOutputsFailed),
            (format!("{:?}", AbortReason::LockInputsOutputsFailed).as_str(), AbortReason::LockInputsOutputsFailed),
            (format!("{:?}", AbortReason::LeaderProposalVsLocalDecisionMismatch).as_str(), AbortReason::LeaderProposalVsLocalDecisionMismatch),
        ]);

        for reason in AbortReason::

        for (reason_str, reason) in possible_matches {
            if s == reason_str.as_str() {
                return Ok();
            }
        }

        Err(FromStrConversionError::InvalidAbortReason(s.to_string()))
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum FromStrConversionError {
    #[error("Invalid Decision string '{0}'")]
    InvalidDecision(String),
    #[error("Invalid Abort reason string '{0}'")]
    InvalidAbortReason(String),
}

impl From<&TransactionResult> for Decision {
    fn from(result: &TransactionResult) -> Self {
        if result.is_accept() {
            Decision::Commit
        } else {
            Decision::Abort
        }
    }
}
