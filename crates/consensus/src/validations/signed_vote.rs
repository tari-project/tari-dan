//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus_types::{SignedMessage, TimeoutVoteMessage, ToSignatureMessage, ValidatorSignatureBytes};
use tari_sidechain::{FixedHash, ProposalVoteMessage};
use tari_template_lib_types::crypto::{RistrettoPublicKeyBytes, SchnorrSignatureBytes};

pub struct SignedProposalVote<'a> {
    pub message: ProposalVoteMessage<'a>,
    pub signature: &'a ValidatorSignatureBytes,
}

impl ToSignatureMessage for SignedProposalVote<'_> {
    fn to_signature_message(&self) -> FixedHash {
        self.message.to_signature_message()
    }
}

impl SignedMessage for SignedProposalVote<'_> {
    fn signature(&self) -> &SchnorrSignatureBytes {
        &self.signature.signature
    }

    fn public_key(&self) -> &RistrettoPublicKeyBytes {
        &self.signature.public_key
    }
}

pub struct SignedTimeoutVote<'a> {
    pub message: TimeoutVoteMessage,
    pub signature: &'a ValidatorSignatureBytes,
}

impl ToSignatureMessage for SignedTimeoutVote<'_> {
    fn to_signature_message(&self) -> FixedHash {
        self.message.to_signature_message()
    }
}

impl SignedMessage for SignedTimeoutVote<'_> {
    fn signature(&self) -> &SchnorrSignatureBytes {
        &self.signature.signature
    }

    fn public_key(&self) -> &RistrettoPublicKeyBytes {
        &self.signature.public_key
    }
}
