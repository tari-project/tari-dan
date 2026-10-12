//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

/// One kind of data that epoch GC prunes once it ages past the retention window.
///
/// Each step is driven by an epoch-ordered index whose entries are deleted in the same write as the rows they point
/// to, so a step can be run in many small transactions and resumes where the previous one stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochCleanupStep {
    /// Clears the values of substates downed in pruned epochs, keeping their records.
    DownedSubstateValues,
    /// Deletes the down proofs of substates downed in pruned epochs. Its window is the down proof retention window,
    /// not the one the other steps share.
    SubstateDownProofs,
    Blocks,
    ProposalCertificates,
    TimeoutCertificates,
    VoteEquivocations,
    ValidatorLivenessLog,
    ForeignProposals,
    /// Only runs when transaction history pruning is enabled.
    FinalizedTransactions,
}

impl EpochCleanupStep {
    pub const ALL: [Self; 9] = [
        Self::DownedSubstateValues,
        Self::SubstateDownProofs,
        Self::Blocks,
        Self::ProposalCertificates,
        Self::TimeoutCertificates,
        Self::VoteEquivocations,
        Self::ValidatorLivenessLog,
        Self::ForeignProposals,
        Self::FinalizedTransactions,
    ];

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::DownedSubstateValues => "downed substate values",
            Self::SubstateDownProofs => "substate down proofs",
            Self::Blocks => "blocks",
            Self::ProposalCertificates => "proposal certificates",
            Self::TimeoutCertificates => "timeout certificates",
            Self::VoteEquivocations => "vote equivocations",
            Self::ValidatorLivenessLog => "validator liveness log entries",
            Self::ForeignProposals => "foreign proposals",
            Self::FinalizedTransactions => "finalized transactions",
        }
    }
}

impl Display for EpochCleanupStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
