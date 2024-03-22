//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_crypto::{
    commitment::HomomorphicCommitmentFactory,
    ristretto::{pedersen::PedersenCommitment, RistrettoPublicKey, RistrettoSecretKey},
};
use tari_engine_types::confidential::get_commitment_factory;
use tari_template_lib::models::{Amount, EncryptedData};

#[derive(Debug, Clone)]
pub struct ConfidentialProofStatement {
    pub amount: Amount,
    pub mask: RistrettoSecretKey,
    pub sender_public_nonce: RistrettoPublicKey,
    pub minimum_value_promise: u64,
    pub encrypted_data: EncryptedData,
    pub reveal_amount: Amount,
    pub resource_view_key: Option<RistrettoPublicKey>,
}

impl ConfidentialProofStatement {
    pub fn to_commitment(&self) -> PedersenCommitment {
        get_commitment_factory().commit_value(&self.mask, self.amount.value() as u64)
    }
}
