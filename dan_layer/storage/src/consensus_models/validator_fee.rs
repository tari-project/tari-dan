//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};
use tari_dan_common_types::{Epoch, NodeAddressable};

use crate::{consensus_models::BlockId, StateStoreReadTransaction, StateStoreWriteTransaction, StorageError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorFee<TAddr> {
    pub validator_addr: TAddr,
    pub epoch: Epoch,
    pub block_id: BlockId,
    pub total_fee_due: u64,
    pub total_transaction_fee: u64,
}

impl<TAddr: NodeAddressable> ValidatorFee<TAddr> {
    pub fn create<TTx: StateStoreWriteTransaction<Addr = TAddr>>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.validator_fees_insert(self)
    }

    pub fn get_total_due_for_epoch<TTx: StateStoreReadTransaction<Addr = TAddr>>(
        tx: &mut TTx,
        epoch: Epoch,
        validator_public_key: &TAddr,
    ) -> Result<u64, StorageError> {
        tx.validator_fees_get_total_fee_for_epoch(epoch, validator_public_key)
    }

    pub fn get_any_with_epoch_range_for_validator<TTx: StateStoreReadTransaction<Addr = TAddr>>(
        tx: &mut TTx,
        range: RangeInclusive<Epoch>,
        validator_public_key: Option<&TAddr>,
    ) -> Result<Vec<Self>, StorageError> {
        tx.validator_fees_get_any_with_epoch_range(range, validator_public_key)
    }
}
