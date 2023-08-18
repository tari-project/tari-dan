//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use log::*;
use tari_dan_common_types::ShardId;
use tari_dan_storage::{
    consensus_models::{ExecutedTransaction, SubstateRecord},
    StateStore,
};

use crate::p2p::services::mempool::{MempoolError, Validator};

const LOG_TARGET: &str = "tari::dan::mempool::validators::outputs_dont_exist";

/// Refuse to process the transaction if any input_refs are downed
pub struct OutputsDontExistLocally<TStateStore> {
    store: TStateStore,
}

impl<TStateStore> OutputsDontExistLocally<TStateStore> {
    pub fn new(store: TStateStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl<TStateStore> Validator<ExecutedTransaction> for OutputsDontExistLocally<TStateStore>
where TStateStore: StateStore + Send + Sync
{
    type Error = MempoolError;

    async fn validate(&self, executed: &ExecutedTransaction) -> Result<(), Self::Error> {
        let Some(diff) = executed.result().finalize.result.accept().cloned() else {
            return Ok(());
        };

        let shards = diff.up_iter().map(|(s, v)| ShardId::from_address(s, v.version()));

        if self.store.with_read_tx(|tx| SubstateRecord::any_exist(tx, shards))? {
            info!(target: LOG_TARGET, "OutputsDontExistLocally - FAIL");
            return Err(MempoolError::OutputSubstateExists {
                transaction_id: *executed.transaction().id(),
            });
        }

        info!(target: LOG_TARGET, "OutputsDontExistLocally - OK");
        Ok(())
    }
}