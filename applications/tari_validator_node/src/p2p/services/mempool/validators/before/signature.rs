//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use log::warn;
use tari_transaction::Transaction;

use crate::p2p::services::mempool::{MempoolError, Validator};

const LOG_TARGET: &str = "tari::dan::mempool::validators::signature";

#[derive(Debug)]
pub struct TransactionSignatureValidator;

#[async_trait]
impl Validator<Transaction> for TransactionSignatureValidator {
    type Error = MempoolError;

    async fn validate(&self, transaction: &Transaction) -> Result<(), MempoolError> {
        if !transaction.signature().verify(&transaction.into()) {
            warn!(target: LOG_TARGET, "TransactionSignatureValidator - FAIL: Invalid signature");
            return Err(MempoolError::InvalidSignature);
        }

        Ok(())
    }
}
