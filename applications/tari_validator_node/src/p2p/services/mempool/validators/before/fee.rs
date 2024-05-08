//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use log::warn;
use tari_transaction::Transaction;

use crate::p2p::services::mempool::{MempoolError, Validator};

const LOG_TARGET: &str = "tari::dan::mempool::validators::fee";

#[derive(Debug)]
pub struct FeeTransactionValidator;

#[async_trait]
impl Validator<Transaction> for FeeTransactionValidator {
    type Error = MempoolError;

    async fn validate(&self, transaction: &Transaction) -> Result<(), MempoolError> {
        if transaction.fee_instructions().is_empty() {
            warn!(target: LOG_TARGET, "FeeTransactionValidator - FAIL: No fee instructions");
            return Err(MempoolError::NoFeeInstructions);
        }
        Ok(())
    }
}
