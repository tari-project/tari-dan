// Copyright 2025 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use log::warn;
use tari_ootle_transaction::Transaction;

use crate::{TransactionValidationError, Validator};

const LOG_TARGET: &str = "tari::ootle::mempool::validators::dry_run";

#[derive(Debug)]
pub struct TransactionDryRunValidator;

impl Validator<Transaction> for TransactionDryRunValidator {
    type Context = ();
    type Error = TransactionValidationError;

    fn validate(&self, _context: &Self::Context, input: &Transaction) -> Result<(), Self::Error> {
        match input {
            Transaction::V1(tx) => {
                if tx.is_dry_run() {
                    warn!(target: LOG_TARGET, "TransactionDryRunValidator - FAIL: dry run transactions are not allowed!");
                    return Err(Self::Error::DryRunNotAllowed);
                }

                Ok(())
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use indexmap::IndexSet;
    use tari_ootle_common_types::Epoch;
    use tari_ootle_transaction::{
        Network,
        Sr25519SignatureBytes,
        Transaction,
        TransactionSealSignature,
        TransactionSignature,
        UnsealedTransactionV1,
        UnsignedTransactionV1,
    };
    use tari_template_lib::types::crypto::RistrettoPublicKeyBytes;

    use crate::{TransactionDryRunValidator, TransactionValidationError, Validator};

    fn tx(dry_run: bool) -> Transaction {
        Transaction::new(
            UnsealedTransactionV1::new(
                UnsignedTransactionV1::new(
                    Network::LocalNet.as_byte(),
                    vec![],
                    vec![],
                    IndexSet::new(),
                    None,
                    Epoch(1),
                    dry_run,
                ),
                vec![TransactionSignature::new(
                    RistrettoPublicKeyBytes::zero(),
                    Sr25519SignatureBytes::zero(),
                )],
            )
            .into(),
            TransactionSealSignature::new(RistrettoPublicKeyBytes::zero(), Sr25519SignatureBytes::zero()),
        )
    }

    #[test]
    fn dry_run_tx_not_allowed() {
        let validator = TransactionDryRunValidator {};
        let tx = tx(true);
        let result = validator.validate(&(), &tx);
        assert!(result.is_err());
        assert!(matches!(
            result.err().unwrap(),
            TransactionValidationError::DryRunNotAllowed
        ));
    }

    #[test]
    fn non_dry_run_tx_allowed() {
        let validator = TransactionDryRunValidator {};
        let tx = tx(false);
        let result = validator.validate(&(), &tx);
        assert!(result.is_ok());
    }
}
