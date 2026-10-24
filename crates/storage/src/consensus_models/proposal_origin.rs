//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use minicbor::{CborLen, Decode, Encode};
use serde::{Deserialize, Serialize};

/// How a local block reached this node, which decides whether this node votes on it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Encode, Decode, CborLen)]
pub enum ProposalOrigin {
    /// A proposal for the current view. This node votes on it.
    #[default]
    #[n(0)]
    Live,
    /// A block imported through catch-up sync. The committee has already certified it, so a vote can never help form
    /// a certificate and this node does not vote on it.
    #[n(1)]
    CatchUp,
}

impl ProposalOrigin {
    pub fn is_catch_up(&self) -> bool {
        matches!(self, Self::CatchUp)
    }
}
