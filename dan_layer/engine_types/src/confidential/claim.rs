//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_bor::{borsh, Encode};
use tari_crypto::ristretto::RistrettoComSig;
use tari_template_lib::models::LayerOneCommitmentAddress;

#[derive(Debug, Clone, Encode, Deserialize, Serialize, Eq, PartialEq)]
pub struct ConfidentialClaim {
    pub commitment_address: LayerOneCommitmentAddress,
    pub range_proof: Vec<u8>,
    pub proof_of_knowledge: RistrettoComSig,
}
