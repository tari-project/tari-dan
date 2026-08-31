//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::FixedHash;
use tari_ootle_common_types::{Epoch, NodeHeight};
use tari_sidechain::QuorumDecision;
use tari_template_lib::types::crypto::{RistrettoPublicKeyBytes, SchnorrSignatureBytes};

pub trait ToSignatureMessage {
    fn to_signature_message(&self) -> FixedHash;
}

/// A message that carries a validator signature. The signed bytes are not part of this trait: a message whose
/// preimage depends on context the message itself does not carry (such as the protocol version a proposal vote was
/// cast under) implements only this, and is paired with its preimage at the point of verification.
pub trait SignedMessage {
    fn signature(&self) -> &SchnorrSignatureBytes;
    fn public_key(&self) -> &RistrettoPublicKeyBytes;
}

pub trait Vote: SignedMessage {
    /// Identifies which votes aggregate together within an (epoch, height) bucket.
    /// Proposal votes aggregate per voted block; timeout votes aggregate per (epoch, height).
    type AggregationKey: PartialEq;

    fn epoch(&self) -> Epoch;
    fn height(&self) -> NodeHeight;
    fn decision(&self) -> QuorumDecision;
    fn aggregation_key(&self) -> Self::AggregationKey;
}
