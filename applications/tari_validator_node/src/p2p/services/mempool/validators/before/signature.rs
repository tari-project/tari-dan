//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use tari_transaction::Transaction;

use crate::p2p::services::mempool::{MempoolError, Validator};

#[derive(Debug)]
pub struct TransactionSignatureValidator;

#[async_trait]
impl Validator<Transaction> for TransactionSignatureValidator {
    type Error = MempoolError;

    async fn validate(&self, transaction: &Transaction) -> Result<(), MempoolError> {
        if !transaction.signature().verify(&transaction.into()) {
            return Err(MempoolError::InvalidSignature);
        }

        Ok(())
    }
}
