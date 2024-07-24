//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use log::warn;
use tari_transaction::Transaction;

use crate::{transaction_validators::TransactionValidationError, validator::Validator};

const LOG_TARGET: &str = "tari::dan::mempool::validators::signature";

#[derive(Debug)]
pub struct TransactionSignatureValidator;

impl Validator<Transaction> for TransactionSignatureValidator {
    type Context = ();
    type Error = TransactionValidationError;

    fn validate(&self, _context: &(), transaction: &Transaction) -> Result<(), TransactionValidationError> {
        if transaction.signatures().is_empty() {
            warn!(target: LOG_TARGET, "TransactionSignatureValidator - FAIL: No signatures");
            return Err(TransactionValidationError::TransactionNotSigned {
                transaction_id: *transaction.id(),
            });
        }

        if !transaction.verify_all_signatures() {
            warn!(target: LOG_TARGET, "TransactionSignatureValidator - FAIL: Invalid signature");
            return Err(TransactionValidationError::InvalidSignature);
        }

        Ok(())
    }
}
