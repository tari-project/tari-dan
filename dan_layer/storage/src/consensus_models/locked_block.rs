//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::DerefMut;

use tari_dan_common_types::{Epoch, NodeHeight};

use crate::{consensus_models::BlockId, StateStoreReadTransaction, StorageError};

pub struct LockedBlock {
    pub height: NodeHeight,
    pub block_id: BlockId,
}

impl LockedBlock {
    pub fn get<TTx>(tx: &mut TTx, epoch: Epoch) -> Result<Self, StorageError>
    where
        TTx: DerefMut,
        TTx::Target: StateStoreReadTransaction,
    {
        let (height, block_id) = tx.deref_mut().locked_block_get(epoch)?;
        Ok(Self {
            height,
            block_id: block_id.into(),
        })
    }
}
