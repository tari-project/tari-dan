//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use serde::Serialize;
use tari_consensus_types::{ProposalVote, SignedMessage};
use tari_template_lib_types::crypto::{RistrettoPublicKeyBytes, SchnorrSignatureBytes};

#[derive(Debug, Clone, Serialize)]
pub struct VoteMessage {
    pub vote: ProposalVote,
}

impl Display for VoteMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.vote)
    }
}

impl SignedMessage for VoteMessage {
    fn signature(&self) -> &SchnorrSignatureBytes {
        self.vote.signature()
    }

    fn public_key(&self) -> &RistrettoPublicKeyBytes {
        self.vote.public_key()
    }
}

impl From<ProposalVote> for VoteMessage {
    fn from(vote: ProposalVote) -> Self {
        Self { vote }
    }
}
