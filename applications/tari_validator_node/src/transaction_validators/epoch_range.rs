//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use log::warn;
use tari_dan_common_types::Epoch;
use tari_transaction::Transaction;

use crate::{transaction_validators::TransactionValidationError, validator::Validator};

const LOG_TARGET: &str = "tari::dan::mempool::validators::epoch_range";

#[derive(Debug, Default)]
pub struct EpochRangeValidator;

impl EpochRangeValidator {
    pub fn new() -> Self {
        Self
    }
}

impl Validator<Transaction> for EpochRangeValidator {
    type Context = Epoch;
    type Error = TransactionValidationError;

    fn validate(&self, &current_epoch: &Epoch, transaction: &Transaction) -> Result<(), TransactionValidationError> {
        if let Some(min_epoch) = transaction.min_epoch() {
            if current_epoch < min_epoch {
                warn!(target: LOG_TARGET, "EpochRangeValidator - FAIL: Current epoch {current_epoch} less than minimum epoch {min_epoch}.");
                return Err(TransactionValidationError::CurrentEpochLessThanMinimum {
                    current_epoch,
                    min_epoch,
                });
            }
        }

        if let Some(max_epoch) = transaction.max_epoch() {
            if current_epoch > max_epoch {
                warn!(target: LOG_TARGET, "EpochRangeValidator - FAIL: Current epoch {current_epoch} greater than maximum epoch {max_epoch}.");
                return Err(TransactionValidationError::CurrentEpochGreaterThanMaximum {
                    current_epoch,
                    max_epoch,
                });
            }
        }

        Ok(())
    }
}
