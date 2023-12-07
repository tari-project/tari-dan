//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use tari_transaction::{Transaction, TransactionSignatureFields};

use crate::p2p::services::mempool::{MempoolError, Validator};

#[derive(Debug)]
pub struct TransactionSignatureValidator;

#[async_trait]
impl Validator<Transaction> for TransactionSignatureValidator {
    type Error = MempoolError;

    async fn validate(&self, transaction: &Transaction) -> Result<(), MempoolError> {
        let signature_fields = TransactionSignatureFields {
            fee_instructions: transaction.fee_instructions().to_vec(),
            instructions: transaction.instructions().to_vec(),
            inputs: transaction.inputs().to_vec(),
            input_refs: transaction.input_refs().to_vec(),
            min_epoch: transaction.min_epoch(),
            max_epoch: transaction.max_epoch(),
        };

        if !transaction.signature().verify(signature_fields) {
            return Err(MempoolError::InvalidSignature);
        }

        Ok(())
    }
}
