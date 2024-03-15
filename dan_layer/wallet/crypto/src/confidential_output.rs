//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_crypto::ristretto::RistrettoSecretKey;

#[derive(Debug, Clone)]
pub struct ConfidentialOutputMaskAndValue {
    pub value: u64,
    pub mask: RistrettoSecretKey,
}
