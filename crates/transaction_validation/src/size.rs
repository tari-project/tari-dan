// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use log::warn;
use tari_ootle_transaction::Transaction;

use crate::{TransactionValidationError, Validator};

const LOG_TARGET: &str = "tari::ootle::mempool::validators::size";

/// Rejects transactions whose encoded size exceeds `max_size_bytes`.
///
/// [`TransactionWeightValidator`](crate::TransactionWeightValidator) bounds work rather than bytes:
/// blob payloads are charged at a divisor, so a transaction can carry several times the weight cap's
/// worth of bytes and still be admitted. Without a byte bound the largest admissible transaction is
/// only implied — by weight arithmetic — which leaves the gossip layer nothing to size its message
/// limit against.
///
/// That relationship is the reason this runs at ingress rather than being left to gossipsub's own
/// frame limit: a message over that limit fails to decode, which costs the sender's peer score and
/// tells nobody why. `ConsensusConstants::max_transaction_size_bytes` is what
/// `tari_ootle_p2p::max_gossip_message_size` is derived from, so a transaction this validator admits
/// is one the mesh will carry.
///
/// The size is `Transaction::encoded_size` — the canonical CBOR length, which is exactly what the
/// p2p message carries and the state store persists. It is a pure function of the transaction, so
/// every node reaches the same verdict.
#[derive(Debug, Clone)]
pub struct TransactionSizeValidator {
    max_size_bytes: usize,
}

impl TransactionSizeValidator {
    pub fn new(max_size_bytes: usize) -> Self {
        Self { max_size_bytes }
    }
}

impl Validator<Transaction> for TransactionSizeValidator {
    type Context = ();
    type Error = TransactionValidationError;

    fn validate(&self, _context: &(), transaction: &Transaction) -> Result<(), Self::Error> {
        let size = transaction.encoded_size();
        if size > self.max_size_bytes {
            let transaction_id = transaction.calculate_id();
            warn!(
                target: LOG_TARGET,
                "TransactionSizeValidator - FAIL: {transaction_id} is {size} bytes, exceeding maximum {}",
                self.max_size_bytes
            );
            return Err(TransactionValidationError::TransactionExceedsMaxSize {
                transaction_id,
                size,
                max_size: self.max_size_bytes,
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use indexmap::IndexSet;
    use tari_ootle_common_types::Epoch;
    use tari_ootle_transaction::{
        Blob,
        Blobs,
        Instruction,
        Network,
        TransactionSealSignature,
        TransactionSignature,
        UnsealedTransactionV1,
        UnsignedTransactionV1,
        args::InstructionArg,
    };
    use tari_template_lib::types::{
        FunctionName,
        TemplateAddress,
        crypto::{RistrettoPublicKeyBytes, SchnorrSignatureBytes},
    };

    use super::*;

    /// A transaction carrying `blob_len` bytes of blob payload, referenced so it passes
    /// `BlobReferenceValidator` — the shape that the weight cap alone does not bound.
    fn tx_with_blob(blob_len: usize) -> Transaction {
        let instructions = vec![Instruction::CallFunction {
            address: TemplateAddress::from_array([0; 32]),
            function: FunctionName::try_from("f").unwrap(),
            args: vec![InstructionArg::Blob(0)],
        }];
        let mut unsigned = UnsignedTransactionV1::new(
            Network::LocalNet.as_byte(),
            vec![],
            instructions,
            IndexSet::new(),
            None,
            Epoch(100),
            false,
        );
        unsigned.blobs = Blobs::from_vec(vec![Blob::from(vec![0u8; blob_len])]);
        Transaction::new(
            UnsealedTransactionV1::new(unsigned, vec![TransactionSignature::new(
                RistrettoPublicKeyBytes::zero(),
                SchnorrSignatureBytes::zero(),
            )])
            .into(),
            TransactionSealSignature::new(RistrettoPublicKeyBytes::zero(), SchnorrSignatureBytes::zero()),
        )
    }

    #[test]
    fn accepts_a_transaction_within_the_size_limit() {
        let tx = tx_with_blob(1024);
        let validator = TransactionSizeValidator::new(64 * 1024);
        assert!(validator.validate(&(), &tx).is_ok());
    }

    #[test]
    fn rejects_a_transaction_over_the_size_limit() {
        let tx = tx_with_blob(64 * 1024);
        let validator = TransactionSizeValidator::new(16 * 1024);
        let err = validator.validate(&(), &tx).unwrap_err();
        assert!(matches!(err, TransactionValidationError::TransactionExceedsMaxSize {
            max_size: 16_384,
            ..
        }));
    }

    /// The reason this validator exists: blob bytes are charged at a divisor, so a transaction the
    /// weight cap admits can carry several times that many bytes. Only the byte cap sees them.
    #[test]
    fn catches_blob_payload_the_weight_cap_admits() {
        let blob_len = 64 * 1024;
        let tx = tx_with_blob(blob_len);

        let weight = tx.calculate_transaction_weight().as_u64() as usize;
        assert!(
            weight < blob_len,
            "blob bytes are meant to weigh less than they measure ({weight} vs {blob_len})"
        );

        // A weight cap generous enough to admit this transaction still leaves its bytes unbounded.
        assert!(
            crate::TransactionWeightValidator::new(weight as u64)
                .validate(&(), &tx)
                .is_ok()
        );
        assert!(TransactionSizeValidator::new(blob_len / 2).validate(&(), &tx).is_err());
    }
}
