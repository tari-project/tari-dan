//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use borsh::BorshSerialize;
use serde::Serialize;
use tari_ootle_common_types::{Epoch, NodeHeight};

use crate::{ProposalCertificate, QcId, TimeoutCertificate};

#[derive(Debug, Clone, Serialize, BorshSerialize)]
pub enum QuorumCertificateRef<'a> {
    ProposalCertificate(&'a ProposalCertificate),
    TimeoutCertificate(&'a TimeoutCertificate),
}

impl QuorumCertificateRef<'_> {
    pub fn is_proposal_certificate(&self) -> bool {
        matches!(self, Self::ProposalCertificate(_))
    }

    pub fn is_timeout_certificate(&self) -> bool {
        matches!(self, Self::TimeoutCertificate(_))
    }

    pub fn justifies_zero_block(&self) -> bool {
        match self {
            Self::ProposalCertificate(pc) => pc.justifies_zero_block(),
            Self::TimeoutCertificate(tc) => tc.height().is_zero(),
        }
    }

    pub fn num_signatures(&self) -> usize {
        match self {
            Self::ProposalCertificate(pc) => pc.signatures().num_signatures(),
            Self::TimeoutCertificate(tc) => tc.num_signatures(),
        }
    }

    pub fn calculate_id(&self) -> QcId {
        match self {
            Self::ProposalCertificate(pc) => pc.calculate_id().into(),
            Self::TimeoutCertificate(tc) => tc.calculate_id().into(),
        }
    }

    pub fn type_str(&self) -> &'static str {
        match self {
            Self::ProposalCertificate(_) => "ProposalCertificate",
            Self::TimeoutCertificate(_) => "TimeoutCertificate",
        }
    }

    pub fn epoch(&self) -> Epoch {
        match self {
            Self::ProposalCertificate(pc) => pc.epoch(),
            Self::TimeoutCertificate(tc) => tc.epoch(),
        }
    }

    pub fn height(&self) -> NodeHeight {
        match self {
            Self::ProposalCertificate(pc) => pc.height(),
            Self::TimeoutCertificate(tc) => tc.height(),
        }
    }

    pub fn as_proposal_certificate(&self) -> Option<&ProposalCertificate> {
        if let Self::ProposalCertificate(pc) = self {
            Some(pc)
        } else {
            None
        }
    }

    pub fn as_timeout_certificate(&self) -> Option<&TimeoutCertificate> {
        if let Self::TimeoutCertificate(tc) = self {
            Some(tc)
        } else {
            None
        }
    }
}

impl<'a> From<&'a ProposalCertificate> for QuorumCertificateRef<'a> {
    fn from(pc: &'a ProposalCertificate) -> Self {
        Self::ProposalCertificate(pc)
    }
}

impl<'a> From<&'a TimeoutCertificate> for QuorumCertificateRef<'a> {
    fn from(tc: &'a TimeoutCertificate) -> Self {
        Self::TimeoutCertificate(tc)
    }
}

impl std::fmt::Display for QuorumCertificateRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProposalCertificate(pc) => write!(f, "{}", pc),
            Self::TimeoutCertificate(tc) => write!(f, "{}", tc),
        }
    }
}
