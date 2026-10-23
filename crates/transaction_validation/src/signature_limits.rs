// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use log::warn;
use tari_ootle_transaction::{MAX_SIGNATURES_PER_TRANSACTION, Transaction};

use crate::{TransactionValidationError, Validator};

const LOG_TARGET: &str = "tari::ootle::mempool::validators::signature_limits";

/// Rejects transactions carrying more than [`MAX_SIGNATURES_PER_TRANSACTION`] authorization signatures.
///
/// Verifying them is work every node that receives the transaction performs, charged to nobody: the set is checked as
/// one batch, whose cost still grows with the count. This is a count-only check, so it must be ordered before
/// [`crate::TransactionSignatureValidator`] to bound the verification work that validator performs.
#[derive(Debug, Clone, Default)]
pub struct SignatureLimitValidator;

impl SignatureLimitValidator {
    pub fn new() -> Self {
        Self
    }
}

impl Validator<Transaction> for SignatureLimitValidator {
    type Context = ();
    type Error = TransactionValidationError;

    fn validate(&self, _context: &(), transaction: &Transaction) -> Result<(), Self::Error> {
        let count = transaction.signatures().len();
        if count > MAX_SIGNATURES_PER_TRANSACTION {
            let transaction_id = transaction.calculate_id();
            warn!(
                target: LOG_TARGET,
                "SignatureLimitValidator - FAIL: {transaction_id} has {count} signatures, maximum is \
                 {MAX_SIGNATURES_PER_TRANSACTION}"
            );
            return Err(TransactionValidationError::TooManySignatures {
                transaction_id,
                max: MAX_SIGNATURES_PER_TRANSACTION,
                actual: count,
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
        Network,
        Sr25519SignatureBytes,
        TransactionSealSignature,
        TransactionSignature,
        UnsealedTransactionV1,
        UnsignedTransactionV1,
    };
    use tari_template_lib::types::crypto::RistrettoPublicKeyBytes;

    use super::*;

    fn tx_with_signatures(num_signatures: usize) -> Transaction {
        Transaction::new(
            UnsealedTransactionV1::new(
                UnsignedTransactionV1::new(
                    Network::LocalNet.as_byte(),
                    vec![],
                    vec![],
                    IndexSet::new(),
                    None,
                    Epoch(1),
                    false,
                ),
                (0..num_signatures)
                    .map(|_| TransactionSignature::new(RistrettoPublicKeyBytes::zero(), Sr25519SignatureBytes::zero()))
                    .collect(),
            )
            .into(),
            TransactionSealSignature::new(RistrettoPublicKeyBytes::zero(), Sr25519SignatureBytes::zero()),
        )
    }

    #[test]
    fn accepts_a_single_signature() {
        SignatureLimitValidator::new()
            .validate(&(), &tx_with_signatures(1))
            .unwrap();
    }

    #[test]
    fn accepts_signatures_at_the_limit() {
        SignatureLimitValidator::new()
            .validate(&(), &tx_with_signatures(MAX_SIGNATURES_PER_TRANSACTION))
            .unwrap();
    }

    #[test]
    fn rejects_signatures_over_the_limit() {
        let over_limit = MAX_SIGNATURES_PER_TRANSACTION + 1;
        let err = SignatureLimitValidator::new()
            .validate(&(), &tx_with_signatures(over_limit))
            .unwrap_err();
        assert!(matches!(
            err,
            TransactionValidationError::TooManySignatures { max, actual, .. }
            if max == MAX_SIGNATURES_PER_TRANSACTION && actual == over_limit
        ));
    }
}
