//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt,
    fmt::{Display, Formatter},
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use tari_engine_types::commit_result::TransactionResult;
#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub enum Decision {
    /// Decision to COMMIT the transaction
    Commit,
    /// Decision to ABORT the transaction
    Abort,
}

impl Decision {
    pub fn is_commit(&self) -> bool {
        matches!(self, Decision::Commit)
    }

    pub fn is_abort(&self) -> bool {
        matches!(self, Decision::Abort)
    }

    pub fn and(self, other: Self) -> Self {
        match self {
            Decision::Commit => other,
            Decision::Abort => Decision::Abort,
        }
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            Decision::Commit => "Commit",
            Decision::Abort => "Abort",
        }
    }
}

impl Display for Decision {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Decision {
    type Err = DecisionFromStrErr;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Commit" => Ok(Decision::Commit),
            "Abort" => Ok(Decision::Abort),
            _ => Err(DecisionFromStrErr(s.to_string())),
        }
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("Invalid Decision string '{0}'")]
pub struct DecisionFromStrErr(String);

impl From<&TransactionResult> for Decision {
    fn from(result: &TransactionResult) -> Self {
        if result.is_accept() {
            Decision::Commit
        } else {
            Decision::Abort
        }
    }
}
