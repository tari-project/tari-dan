//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use log::*;
use tari_dan_common_types::Epoch;
use tari_engine_types::instruction::Instruction;
use tari_epoch_manager::EpochManagerReader;
use tari_transaction::Transaction;

use crate::p2p::services::mempool::{MempoolError, Validator};

const LOG_TARGET: &str = "tari::dan::mempool::validators::claim_fee_instructions";

#[derive(Debug)]
pub struct ClaimFeeTransactionValidator<TEpochManager> {
    epoch_manager: TEpochManager,
}

impl<TEpochManager> ClaimFeeTransactionValidator<TEpochManager> {
    pub fn new(epoch_manager: TEpochManager) -> Self {
        Self { epoch_manager }
    }
}

#[async_trait]
impl<TEpochManager: EpochManagerReader> Validator<Transaction> for ClaimFeeTransactionValidator<TEpochManager> {
    type Error = MempoolError;

    async fn validate(&self, transaction: &Transaction) -> Result<(), MempoolError> {
        let current_epoch = self.epoch_manager.current_epoch().await?;

        let mut claim_fees = transaction
            .fee_instructions()
            .iter()
            .chain(transaction.instructions())
            .filter_map(|i| {
                if let Instruction::ClaimValidatorFees { epoch, .. } = i {
                    Some(epoch)
                } else {
                    None
                }
            });

        if let Some(epoch) = claim_fees.find(|e| **e >= current_epoch.as_u64()) {
            warn!(
                target: LOG_TARGET,
                "ClaimFeeTransactionValidator - FAIL: Rejecting fee claim for epoch {} because it is equal or greater than the current epoch {}",
                epoch,
                current_epoch
            );
            return Err(MempoolError::ValidatorFeeClaimEpochInvalid {
                transaction_id: *transaction.id(),
                given_epoch: Epoch(*epoch),
            });
        }

        Ok(())
    }
}
