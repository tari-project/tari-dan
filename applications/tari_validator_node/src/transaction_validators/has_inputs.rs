//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_engine_types::instruction::Instruction;
use tari_transaction::Transaction;

use crate::{transaction_validators::TransactionValidationError, validator::Validator};

const LOG_TARGET: &str = "tari::dan::mempool::validators::has_involved_shards";

/// Refuse to process the transaction if it does not have any inputs.
/// We make an exception (for now) for CreateFreeTestCoins transactions, which have no inputs.
#[derive(Debug, Clone, Default)]
pub struct HasInputs;

impl HasInputs {
    pub fn new() -> Self {
        Self
    }
}

impl Validator<Transaction> for HasInputs {
    type Context = ();
    type Error = TransactionValidationError;

    fn validate(&self, _context: &(), transaction: &Transaction) -> Result<(), Self::Error> {
        if transaction.all_inputs_iter().next().is_none() {
            // TODO: remove this conditional when we remove CreateFreeTestCoins
            if transaction
                .fee_instructions()
                .iter()
                .any(|i| matches!(i, Instruction::CreateFreeTestCoins { .. }))
            {
                debug!(target: LOG_TARGET, "HasInputs - OK: CreateFreeTestCoins");
                return Ok(());
            }

            warn!(target: LOG_TARGET, "HasInputs - FAIL: No input shards");
            return Err(TransactionValidationError::NoInputs {
                transaction_id: *transaction.id(),
            });
        }

        debug!(target: LOG_TARGET, "HasInputs - OK");
        Ok(())
    }
}
