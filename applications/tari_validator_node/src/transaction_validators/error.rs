//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_dan_app_utilities::template_manager::interface::TemplateManagerError;
use tari_dan_common_types::Epoch;
use tari_dan_storage::{consensus_models::TransactionPoolError, StorageError};
use tari_networking::NetworkingError;
use tari_transaction::TransactionId;

use crate::virtual_substate::VirtualSubstateError;

#[derive(thiserror::Error, Debug)]
pub enum TransactionValidationError {
    #[error("Storage Error: {0}")]
    StorageError(#[from] StorageError),
    #[error("Virtual substate error: {0}")]
    VirtualSubstateError(#[from] VirtualSubstateError),
    #[error("Transaction pool error: {0}")]
    TransactionPoolError(#[from] TransactionPoolError),

    // TODO: move these to MempoolValidationError type
    #[error("Invalid template address: {0}")]
    InvalidTemplateAddress(#[from] TemplateManagerError),
    #[error("No fee instructions")]
    NoFeeInstructions,
    #[error("Output substate exists in transaction {transaction_id}")]
    OutputSubstateExists { transaction_id: TransactionId },
    #[error("Validator fee claim instruction in transaction {transaction_id} contained invalid epoch {given_epoch}")]
    ValidatorFeeClaimEpochInvalid {
        transaction_id: TransactionId,
        given_epoch: Epoch,
    },
    #[error("Current epoch ({current_epoch}) is less than minimum epoch ({min_epoch}) required for transaction")]
    CurrentEpochLessThanMinimum { current_epoch: Epoch, min_epoch: Epoch },
    #[error("Current epoch ({current_epoch}) is greater than maximum epoch ({max_epoch}) required for transaction")]
    CurrentEpochGreaterThanMaximum { current_epoch: Epoch, max_epoch: Epoch },
    #[error("Transaction {transaction_id} does not have any inputs")]
    NoInputs { transaction_id: TransactionId },
    #[error("Executed transaction {transaction_id} does not involved any shards")]
    NoInvolvedShards { transaction_id: TransactionId },
    #[error("Invalid transaction signature")]
    InvalidSignature,
    #[error("Transaction {transaction_id} is not signed")]
    TransactionNotSigned { transaction_id: TransactionId },
    #[error("Network error: {0}")]
    NetworkingError(#[from] NetworkingError),
}
