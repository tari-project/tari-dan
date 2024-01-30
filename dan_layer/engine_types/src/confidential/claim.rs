//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_common_types::types::PublicKey;
use tari_crypto::ristretto::RistrettoComSig;
use tari_template_lib::models::{ConfidentialWithdrawProof, UnclaimedConfidentialOutputAddress};
#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Debug, Clone, Deserialize, Serialize, Eq, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ConfidentialClaim {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_key: PublicKey,
    pub output_address: UnclaimedConfidentialOutputAddress,
    pub range_proof: Vec<u8>,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub proof_of_knowledge: RistrettoComSig,
    pub withdraw_proof: Option<ConfidentialWithdrawProof>,
}
