// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use log::warn;
use tari_ootle_transaction::Transaction;

use crate::{TransactionValidationError, Validator};

const LOG_TARGET: &str = "tari::ootle::mempool::validators::blob_references";

/// Rejects transactions whose blob list does not correspond exactly to what the instructions
/// reference: every `BlobIndex` must be in bounds, and every blob must be referenced.
///
/// Out-of-bounds indices are otherwise only caught during execution, wasting the mempool, gossip
/// and consensus work spent on a transaction that can never succeed. Unreferenced blobs are never
/// caught at all: their bytes are gossiped, stored and retained by every node while contributing
/// nothing to execution, so a transaction can carry arbitrary payload up to the weight cap.
///
/// Both are deterministic properties of the transaction itself, so every honest node reaches the
/// same verdict.
#[derive(Debug, Clone, Default)]
pub struct BlobReferenceValidator;

impl BlobReferenceValidator {
    pub fn new() -> Self {
        Self
    }
}

impl Validator<Transaction> for BlobReferenceValidator {
    type Context = ();
    type Error = TransactionValidationError;

    fn validate(&self, _context: &(), transaction: &Transaction) -> Result<(), Self::Error> {
        if let Err(source) = transaction.validate_blob_references() {
            let transaction_id = transaction.calculate_id();
            warn!(target: LOG_TARGET, "BlobReferenceValidator - FAIL: {transaction_id}: {source}");
            return Err(TransactionValidationError::InvalidBlobReferences { transaction_id, source });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use indexmap::IndexSet;
    use tari_engine_types::Epoch;
    use tari_ootle_transaction::{
        Blob,
        Blobs,
        Instruction,
        Network,
        Sr25519SignatureBytes,
        TransactionSealSignature,
        TransactionSignature,
        UnsealedTransactionV1,
        UnsignedTransactionV1,
        args::InstructionArg,
    };
    use tari_template_lib::types::{FunctionName, TemplateAddress, crypto::RistrettoPublicKeyBytes};

    use super::*;

    fn publish_template(binary: u8) -> Instruction {
        Instruction::PublishTemplate {
            binary,
            metadata_hash: None,
        }
    }

    fn call_function(args: Vec<InstructionArg>) -> Instruction {
        Instruction::CallFunction {
            address: TemplateAddress::from_array([0; 32]),
            function: FunctionName::try_from("f").unwrap(),
            args,
        }
    }

    fn tx(blobs: Vec<Blob>, instructions: Vec<Instruction>) -> Transaction {
        let mut unsigned = UnsignedTransactionV1::new(
            Network::LocalNet.as_byte(),
            vec![],
            instructions,
            IndexSet::new(),
            None,
            Epoch(100),
            false,
        );
        unsigned.blobs = Blobs::from_vec(blobs);
        Transaction::new(
            UnsealedTransactionV1::new(unsigned, vec![TransactionSignature::new(
                RistrettoPublicKeyBytes::zero(),
                Sr25519SignatureBytes::zero(),
            )])
            .into(),
            TransactionSealSignature::new(RistrettoPublicKeyBytes::zero(), Sr25519SignatureBytes::zero()),
        )
    }

    fn validate(transaction: &Transaction) -> Result<(), TransactionValidationError> {
        BlobReferenceValidator::new().validate(&(), transaction)
    }

    #[test]
    fn accepts_a_transaction_with_no_blobs() {
        validate(&tx(vec![], vec![call_function(vec![])])).unwrap();
    }

    #[test]
    fn accepts_every_blob_being_referenced() {
        let transaction = tx(vec![Blob::from(vec![1]), Blob::from(vec![2])], vec![
            publish_template(0),
            call_function(vec![InstructionArg::blob(1)]),
        ]);
        validate(&transaction).unwrap();
    }

    #[test]
    fn rejects_an_out_of_bounds_blob_index() {
        let err = validate(&tx(vec![Blob::from(vec![1])], vec![publish_template(1)])).unwrap_err();
        assert!(
            matches!(err, TransactionValidationError::InvalidBlobReferences { .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_an_unreferenced_blob() {
        let transaction = tx(vec![Blob::from(vec![1]), Blob::from(vec![2])], vec![publish_template(
            0,
        )]);
        let err = validate(&transaction).unwrap_err();
        assert!(
            matches!(err, TransactionValidationError::InvalidBlobReferences { .. }),
            "unexpected error: {err}"
        );
    }
}
