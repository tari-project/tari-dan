//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::info;
use tari_transaction::Transaction;

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::block_transaction_executor";

#[derive(thiserror::Error, Debug)]
pub enum BlockTransactionExecutorError {
    #[error("Placeholder error")]
    PlaceHolderError,
}

#[derive(Debug, Clone)]
pub struct BlockTransactionExecutor {
    
}

impl BlockTransactionExecutor {
    pub fn new() -> Self {
        Self {
            
        }
    }

    pub fn execute(
        &self,
        transaction: &Transaction,
    ) -> Result<(), BlockTransactionExecutorError> {
        info!(
            target: LOG_TARGET,
            "Executing transaction: {}",
            transaction.id(),
        );
        Ok(())
    }
}