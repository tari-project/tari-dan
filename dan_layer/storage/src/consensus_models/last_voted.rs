//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::{Epoch, NodeHeight};

use crate::{StateStoreReadTransaction, StorageError};

pub struct LastVoted {
    pub height: NodeHeight,
}

impl LastVoted {
    pub fn get<TTx: StateStoreReadTransaction>(tx: &mut TTx, epoch: Epoch) -> Result<Self, StorageError> {
        let height = tx.last_vote_height_get(epoch)?;
        Ok(Self { height: height.into() })
    }
}
