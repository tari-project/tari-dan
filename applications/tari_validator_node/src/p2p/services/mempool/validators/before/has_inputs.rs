//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use log::*;
use tari_engine_types::instruction::Instruction;
use tari_transaction::Transaction;

use crate::p2p::services::mempool::{MempoolError, Validator};

const LOG_TARGET: &str = "tari::dan::mempool::validators::has_involved_shards";

/// Refuse to process the transaction if it does not have any inputs.
/// We make an exception (for now) for CreateFreeTestCoins transactions, which have no inputs.
pub struct HasInputs;

impl HasInputs {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Validator<Transaction> for HasInputs {
    type Error = MempoolError;

    async fn validate(&self, transaction: &Transaction) -> Result<(), Self::Error> {
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

            debug!(target: LOG_TARGET, "HasInputs - FAIL: No input shards");
            return Err(MempoolError::NoInputs {
                transaction_id: *transaction.id(),
            });
        }

        debug!(target: LOG_TARGET, "HasInputs - OK");
        Ok(())
    }
}
