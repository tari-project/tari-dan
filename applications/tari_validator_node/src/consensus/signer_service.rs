//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use ootle_byte_type::ConvertFromByteType;
use tari_consensus::traits::{ValidatorSignatureVerifierService, ValidatorSignerService};
use tari_consensus_types::{SignedMessage, ToSignatureMessage, ValidatorSchnorrSignature};
use tari_crypto::ristretto::RistrettoPublicKey;
use tari_ootle_app_utilities::keypair::RistrettoKeypair;

#[derive(Debug, Clone)]
pub struct TariSignatureService {
    keypair: RistrettoKeypair,
}

impl TariSignatureService {
    pub fn new(keypair: RistrettoKeypair) -> Self {
        Self { keypair }
    }
}

impl ValidatorSignerService for TariSignatureService {
    fn sign<M: ToSignatureMessage>(&self, message: &M) -> ValidatorSchnorrSignature {
        ValidatorSchnorrSignature::sign(
            self.keypair.secret_key(),
            message.to_signature_message(),
            &mut rand::rng(),
        )
        .unwrap()
    }

    fn public_key(&self) -> &RistrettoPublicKey {
        self.keypair.public_key()
    }
}

impl ValidatorSignatureVerifierService for TariSignatureService {
    fn verify<M: SignedMessage + ToSignatureMessage>(&self, message: &M) -> bool {
        let Ok(public_key) = RistrettoPublicKey::convert_from_byte_type(message.public_key()) else {
            return false;
        };
        let Ok(signature) = ValidatorSchnorrSignature::convert_from_byte_type(message.signature()) else {
            return false;
        };
        let message = message.to_signature_message();
        signature.verify(&public_key, message)
    }
}
