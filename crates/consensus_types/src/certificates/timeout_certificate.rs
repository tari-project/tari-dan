//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Display, hash::Hash};

use borsh::BorshSerialize;
use minicbor::{CborLen, Decode, Encode};
use serde::{Deserialize, Serialize};
use tari_ootle_common_types::{Epoch, NodeHeight, hashing::timeout_certificate_id_hasher};
use tari_sidechain::{QcSignaturesError, SignerBitmap};
use tari_template_lib::types::crypto::SchnorrSignatureBytes;

use crate::{HighTc, TcId};

/// A committee member's signed timeout for a view, carrying the height of its high proposal certificate. The signer is
/// the member whose bit the certificate's [`SignerBitmap`] sets at this timeout's position.
#[derive(Debug, Clone, Hash, PartialEq, Eq, Deserialize, Serialize, BorshSerialize, Encode, Decode, CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SignedTimeout {
    #[n(0)]
    pub high_pc_height: NodeHeight,
    #[n(1)]
    pub signature: SchnorrSignatureBytes,
}

/// The signatures of a timeout certificate, tagged by the scheme they are made under.
///
/// Each timeout signs its signer's own high proposal certificate height, so the signed messages differ and a scheme
/// that aggregates signatures over one message cannot combine them.
#[derive(Debug, Clone, Hash, PartialEq, Eq, Deserialize, Serialize, BorshSerialize, Encode, Decode, CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum TcSignatures {
    /// One Schnorr-signed timeout per signer, listed in ascending order of the signer's committee index.
    #[n(0)]
    Schnorr {
        #[n(0)]
        #[cbor(with = "tari_bor::adapters::serde_bridge")]
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        signers: SignerBitmap,
        #[n(1)]
        timeouts: Vec<SignedTimeout>,
    },
}

impl TcSignatures {
    pub fn signers(&self) -> &SignerBitmap {
        match self {
            Self::Schnorr { signers, .. } => signers,
        }
    }

    pub fn timeouts(&self) -> &[SignedTimeout] {
        match self {
            Self::Schnorr { timeouts, .. } => timeouts,
        }
    }

    pub fn num_signatures(&self) -> usize {
        self.timeouts().len()
    }

    /// Checks that the signer bitmap is the encoding for a committee of `committee_size` members and that there is
    /// exactly one timeout per signer. Verifies no signature, so this bounds the work a certificate can cause before
    /// any is verified.
    pub fn check_well_formed(&self, committee_size: usize) -> Result<(), QcSignaturesError> {
        self.signers().check_committee_size(committee_size)?;
        let num_signers = self.signers().count();
        let num_signatures = self.num_signatures();
        if num_signers != num_signatures {
            return Err(QcSignaturesError::SignatureCountMismatch {
                num_signers,
                num_signatures,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Hash, Deserialize, Serialize, BorshSerialize, Encode, Decode, CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct TimeoutCertificate {
    #[n(0)]
    epoch: Epoch,
    #[n(1)]
    height: NodeHeight,
    /// A quorum of signed timeouts, each attesting to the height of the signer's high proposal certificate at the time
    /// it timed out.
    #[n(2)]
    signatures: TcSignatures,
}

impl TimeoutCertificate {
    pub fn new(epoch: Epoch, height: NodeHeight, signatures: TcSignatures) -> Self {
        Self {
            epoch,
            height,
            signatures,
        }
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn height(&self) -> NodeHeight {
        self.height
    }

    pub fn signatures(&self) -> &TcSignatures {
        &self.signatures
    }

    pub fn timeouts(&self) -> &[SignedTimeout] {
        self.signatures.timeouts()
    }

    pub fn num_signatures(&self) -> usize {
        self.signatures.num_signatures()
    }

    /// The highest proposal certificate any signer attests to. A block carrying this timeout certificate must
    /// justify from a certificate at least this high, which is what stops a leader from proposing on a certificate
    /// older than the one the committee has already reached.
    pub fn max_high_pc_height(&self) -> NodeHeight {
        self.timeouts()
            .iter()
            .map(|timeout| timeout.high_pc_height)
            .max()
            .unwrap_or_else(NodeHeight::zero)
    }

    pub fn calculate_id(&self) -> TcId {
        timeout_certificate_id_hasher().chain(self).finalize_into_array().into()
    }

    pub fn as_high_tc(&self) -> HighTc {
        HighTc {
            epoch: self.epoch,
            height: self.height,
            tc_id: self.calculate_id(),
        }
    }
}

impl Display for TimeoutCertificate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TimeoutCertificate {{ epoch: {}, height: {}, signatures: {}, max high pc: {} }}",
            self.epoch,
            self.height,
            self.num_signatures(),
            self.max_high_pc_height()
        )
    }
}
