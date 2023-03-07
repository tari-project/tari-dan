//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use chacha20poly1305::aead;
use tari_crypto::errors::RangeProofError;

#[derive(Debug, thiserror::Error)]
pub enum ConfidentialProofError {
    #[error("Range proof error: {0}")]
    RangeProof(#[from] RangeProofError),
    #[error("Aead error")]
    AeadError,
}

impl From<aead::Error> for ConfidentialProofError {
    fn from(_value: aead::Error) -> Self {
        Self::AeadError
    }
}
