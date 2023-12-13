//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use tari_dan_common_types::NodeAddressable;
use tari_epoch_manager::{base_layer::EpochManagerHandle, EpochManagerReader};
use tari_transaction::Transaction;

use crate::p2p::services::mempool::{MempoolError, Validator};

#[derive(Debug)]
pub struct EpochRangeValidator<TAddr> {
    epoch_manager: EpochManagerHandle<TAddr>,
}

impl<TAddr> EpochRangeValidator<TAddr> {
    pub fn new(epoch_manager: EpochManagerHandle<TAddr>) -> Self {
        Self { epoch_manager }
    }
}

#[async_trait]
impl<TAddr: NodeAddressable> Validator<Transaction> for EpochRangeValidator<TAddr> {
    type Error = MempoolError;

    async fn validate(&self, transaction: &Transaction) -> Result<(), MempoolError> {
        let current_epoch = self.epoch_manager.current_epoch().await?;
        if let Some(min_epoch) = transaction.min_epoch() {
            if current_epoch < min_epoch {
                return Err(MempoolError::CurrentEpochLessThanMinimum {
                    current_epoch,
                    min_epoch,
                });
            }
        }

        if let Some(max_epoch) = transaction.max_epoch() {
            if current_epoch > max_epoch {
                return Err(MempoolError::CurrentEpochGreaterThanMaximum {
                    current_epoch,
                    max_epoch,
                });
            }
        }

        Ok(())
    }
}
