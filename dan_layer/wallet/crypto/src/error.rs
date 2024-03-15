//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use chacha20poly1305::aead;
use tari_crypto::errors::RangeProofError;

#[derive(Debug, thiserror::Error)]
pub enum ConfidentialProofError {
    #[error("Range proof error: {0}")]
    RangeProof(RangeProofError),
    #[error("Aead error")]
    AeadError,
    #[error("Negative amount")]
    NegativeAmount,
}

impl From<aead::Error> for ConfidentialProofError {
    fn from(_value: aead::Error) -> Self {
        Self::AeadError
    }
}

impl From<RangeProofError> for ConfidentialProofError {
    fn from(value: RangeProofError) -> Self {
        Self::RangeProof(value)
    }
}
