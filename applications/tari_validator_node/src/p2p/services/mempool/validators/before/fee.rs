//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use tari_transaction::Transaction;

use crate::p2p::services::mempool::{MempoolError, Validator};

#[derive(Debug)]
pub struct FeeTransactionValidator;

#[async_trait]
impl Validator<Transaction> for FeeTransactionValidator {
    type Error = MempoolError;

    async fn validate(&self, transaction: &Transaction) -> Result<(), MempoolError> {
        if transaction.fee_instructions().is_empty() {
            return Err(MempoolError::NoFeeInstructions);
        }
        Ok(())
    }
}
