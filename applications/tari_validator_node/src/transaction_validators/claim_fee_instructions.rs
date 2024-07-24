//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_dan_common_types::Epoch;
use tari_engine_types::instruction::Instruction;
use tari_transaction::Transaction;

use crate::{transaction_validators::error::TransactionValidationError, validator::Validator};

const LOG_TARGET: &str = "tari::dan::validators::claim_fee_instructions";

#[derive(Debug, Default)]
pub struct ClaimFeeTransactionValidator;

impl ClaimFeeTransactionValidator {
    pub fn new() -> Self {
        Self
    }
}

impl Validator<Transaction> for ClaimFeeTransactionValidator {
    type Context = Epoch;
    type Error = TransactionValidationError;

    fn validate(&self, &current_epoch: &Epoch, transaction: &Transaction) -> Result<(), Self::Error> {
        let mut claim_fees = transaction
            .fee_instructions()
            .iter()
            .chain(transaction.instructions())
            .filter_map(|i| {
                if let Instruction::ClaimValidatorFees { epoch, .. } = i {
                    Some(epoch)
                } else {
                    None
                }
            });

        if let Some(&epoch) = claim_fees.find(|e| **e >= current_epoch.as_u64()) {
            warn!(
                target: LOG_TARGET,
                "ClaimFeeTransactionValidator - FAIL: Rejecting fee claim for epoch {} because it is equal or greater than the current epoch {}",
                epoch,
                current_epoch
            );
            return Err(TransactionValidationError::ValidatorFeeClaimEpochInvalid {
                transaction_id: *transaction.id(),
                given_epoch: Epoch(epoch),
            });
        }

        Ok(())
    }
}
