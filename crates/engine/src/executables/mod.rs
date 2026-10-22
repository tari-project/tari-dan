//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod transaction;

use tari_ootle_common_types::InputDeclaration;
use tari_ootle_transaction::{Blobs, Instruction, TransactionId, TransactionWeight};
use tari_template_lib::types::{Hash32, crypto::RistrettoPublicKeyBytes};

pub struct Instructions {
    pub fee: Vec<Instruction>,
    pub main: Vec<Instruction>,
    /// Blob payloads referenced by `BlobIndex` from instructions and args. Resolved by the
    /// processor before executing each instruction.
    pub blobs: Blobs,
}

pub trait Executable {
    fn to_id(&self) -> TransactionId;

    /// Commitment to everything the signers authorized, recorded in the transaction receipt so a
    /// holder of the transaction can link it to the receipt without revealing the signers.
    fn calculate_intent_commitment(&self) -> Hash32;

    /// Both of the above. Implementations whose id and commitment share work — deriving blob
    /// commitments hashes every blob payload — should override this to do that work once.
    fn to_id_and_intent_commitment(&self) -> (TransactionId, Hash32) {
        (self.to_id(), self.calculate_intent_commitment())
    }

    fn all_inputs_iter(&self) -> impl Iterator<Item = InputDeclaration> + '_;

    /// The declared inputs the transaction pays to load and pledge. An executable that declares
    /// more than its transaction carries — a test harness standing in for consensus's input
    /// resolution — prices only the transaction's own declarations.
    fn charged_inputs_iter(&self) -> impl Iterator<Item = InputDeclaration> + '_ {
        self.all_inputs_iter()
    }

    /// Returns the key the executable acts as wherever the engine needs a single signer, if it has one.
    fn main_signer(&self) -> Option<RistrettoPublicKeyBytes> {
        self.signers_iter().next().copied()
    }
    /// Returns an iterator over all signers of the executable including the main signer.
    fn signers_iter(&self) -> impl Iterator<Item = &RistrettoPublicKeyBytes>;

    fn into_instructions(self) -> Instructions;
}

pub trait WeightedExecutable: Executable {
    fn calculate_weight(&self) -> TransactionWeight;
}
