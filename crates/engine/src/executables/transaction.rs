//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_ootle_common_types::InputDeclaration;
use tari_ootle_transaction::{Transaction, TransactionId, TransactionIntent, TransactionWeight};
use tari_template_lib::types::{Hash32, crypto::RistrettoPublicKeyBytes};

use crate::executables::{Executable, Instructions, WeightedExecutable};

impl Executable for Transaction {
    fn to_id(&self) -> TransactionId {
        self.calculate_id()
    }

    fn calculate_intent_commitment(&self) -> Hash32 {
        TransactionIntent::calculate_intent_commitment(self)
    }

    fn to_id_and_intent_commitment(&self) -> (TransactionId, Hash32) {
        self.calculate_id_and_intent_commitment()
    }

    fn all_inputs_iter(&self) -> impl Iterator<Item = InputDeclaration> + '_ {
        self.all_inputs_iter().map(|decl| decl.to_owned())
    }

    fn main_signer(&self) -> Option<RistrettoPublicKeyBytes> {
        Transaction::main_signer(self)
    }

    fn signers_iter(&self) -> impl Iterator<Item = &RistrettoPublicKeyBytes> {
        self.is_seal_signer_authorized()
            .then_some(self.seal_signature().public_key())
            .into_iter()
            .chain(self.signatures().iter().map(|s| s.public_key()))
    }

    fn into_instructions(self) -> Instructions {
        let (fee, main, blobs) = self.into_instructions_and_blobs();
        Instructions { fee, main, blobs }
    }
}

impl WeightedExecutable for Transaction {
    fn calculate_weight(&self) -> TransactionWeight {
        self.calculate_transaction_weight()
    }
}
