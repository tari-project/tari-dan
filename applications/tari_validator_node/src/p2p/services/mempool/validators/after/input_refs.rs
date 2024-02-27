//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use tari_dan_storage::consensus_models::ExecutedTransaction;
use tari_transaction::SubstateRequirement;

use crate::p2p::services::mempool::{MempoolError, Validator};

/// Refuse to process the transaction if any input_refs are downed
pub struct InputRefsValidator;

impl InputRefsValidator {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Validator<ExecutedTransaction> for InputRefsValidator {
    type Error = MempoolError;

    async fn validate(&self, executed: &ExecutedTransaction) -> Result<(), Self::Error> {
        let Some(diff) = executed.result().finalize.result.accept() else {
            return Ok(());
        };

        let is_input_refs_downed = diff
            .down_iter()
            .map(|(s, v)| SubstateRequirement::new(s.clone(), Some(*v)))
            .any(|s| executed.transaction().input_refs().contains(&s));

        if is_input_refs_downed {
            return Err(MempoolError::InputRefsDowned);
        }

        Ok(())
    }
}
