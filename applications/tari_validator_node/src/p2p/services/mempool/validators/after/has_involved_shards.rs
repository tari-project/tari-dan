//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use log::*;
use tari_dan_storage::consensus_models::ExecutedTransaction;

use crate::p2p::services::mempool::{MempoolError, Validator};

const LOG_TARGET: &str = "tari::dan::mempool::validators::has_involved_shards";

/// Refuse to process the transaction if it does not have any involved shards.
/// This may be removed in future in favour of a stricter rule that requires all transactions to have at least one
/// input/input_ref before execution. Currently, we need to allow zero inputs because of CreateFreeTestCoins.
pub struct HasInvolvedShards;

impl HasInvolvedShards {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Validator<ExecutedTransaction> for HasInvolvedShards {
    type Error = MempoolError;

    async fn validate(&self, executed: &ExecutedTransaction) -> Result<(), Self::Error> {
        if executed.num_inputs_and_outputs() == 0 {
            debug!(target: LOG_TARGET, "HasInvolvedShards - FAIL: No input or output shards");
            return Err(MempoolError::NoInvolvedShards {
                transaction_id: *executed.id(),
            });
        }

        Ok(())
    }
}
