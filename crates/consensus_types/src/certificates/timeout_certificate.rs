//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Display, hash::Hash};

use borsh::BorshSerialize;
use minicbor::{CborLen, Decode, Encode};
use serde::{Deserialize, Serialize};
use tari_ootle_common_types::{Epoch, NodeHeight, hashing::timeout_certificate_id_hasher};

use crate::{HighTc, TcId, validator_signature::ValidatorSignatureBytes};

/// A validator's signed timeout for a view, carrying the height of its high proposal certificate.
#[derive(Debug, Clone, Hash, Deserialize, Serialize, BorshSerialize, Encode, Decode, CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SignedTimeout {
    #[n(0)]
    pub high_pc_height: NodeHeight,
    #[n(1)]
    pub signature: ValidatorSignatureBytes,
}

#[derive(Debug, Clone, Hash, Deserialize, Serialize, BorshSerialize, Encode, Decode, CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct TimeoutCertificate {
    #[n(0)]
    epoch: Epoch,
    #[n(1)]
    height: NodeHeight,
    /// A quorum of validator signatures that sign the timeout certificate, each attesting to the height of the
    /// signer's high proposal certificate at the time it timed out.
    #[n(2)]
    timeouts: Vec<SignedTimeout>,
}

impl TimeoutCertificate {
    pub fn new(epoch: Epoch, height: NodeHeight, timeouts: Vec<SignedTimeout>) -> Self {
        Self {
            epoch,
            height,
            timeouts,
        }
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn height(&self) -> NodeHeight {
        self.height
    }

    pub fn timeouts(&self) -> &[SignedTimeout] {
        &self.timeouts
    }

    pub fn num_signatures(&self) -> usize {
        self.timeouts.len()
    }

    /// The highest proposal certificate any signer attests to. A block carrying this timeout certificate must
    /// justify from a certificate at least this high, which is what stops a leader from proposing on a certificate
    /// older than the one the committee has already reached.
    pub fn max_high_pc_height(&self) -> NodeHeight {
        self.timeouts
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
            self.timeouts.len(),
            self.max_high_pc_height()
        )
    }
}
