//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_dan_common_types::{Epoch, NodeAddressable};
use tari_dan_storage::{consensus_models, StorageError};
use time::PrimitiveDateTime;

use crate::serialization::{deserialize_hex, deserialize_hex_try_from};

#[derive(Debug, Clone, Queryable)]
pub struct ValidatorFee {
    pub id: i32,
    pub validator_addr: String,
    pub epoch: i64,
    pub block_id: String,
    pub total_fee_due: i64,
    pub total_transaction_fee: i64,
    pub created_at: PrimitiveDateTime,
}

impl<TAddr: NodeAddressable> TryFrom<ValidatorFee> for consensus_models::ValidatorFee<TAddr> {
    type Error = StorageError;

    fn try_from(value: ValidatorFee) -> Result<Self, Self::Error> {
        Ok(Self {
            validator_addr: TAddr::from_bytes(&deserialize_hex(&value.validator_addr)?).ok_or_else(|| {
                StorageError::DecodingError {
                    operation: "ValidatorFee::try_from",
                    item: "validator_addr",
                    details: "Failed to decode validator address".to_string(),
                }
            })?,
            epoch: Epoch(value.epoch as u64),
            block_id: deserialize_hex_try_from(&value.block_id)?,
            total_fee_due: value.total_fee_due as u64,
            total_transaction_fee: value.total_transaction_fee as u64,
        })
    }
}
