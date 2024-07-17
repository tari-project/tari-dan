//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use log::warn;
use tari_transaction::Transaction;

use crate::{transaction_validators::TransactionValidationError, validator::Validator};

const LOG_TARGET: &str = "tari::dan::mempool::validators::fee";

#[derive(Debug)]
pub struct FeeTransactionValidator;

impl Validator<Transaction> for FeeTransactionValidator {
    type Context = ();
    type Error = TransactionValidationError;

    fn validate(&self, _context: &(), transaction: &Transaction) -> Result<(), TransactionValidationError> {
        if transaction.fee_instructions().is_empty() {
            warn!(target: LOG_TARGET, "FeeTransactionValidator - FAIL: No fee instructions");
            return Err(TransactionValidationError::NoFeeInstructions);
        }
        Ok(())
    }
}
