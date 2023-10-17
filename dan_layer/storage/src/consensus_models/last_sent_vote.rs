//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::{Epoch, NodeHeight};

use super::{QuorumDecision, ValidatorSignature};
use crate::{consensus_models::BlockId, StateStoreReadTransaction, StateStoreWriteTransaction, StorageError};

pub struct LastSentVote<TAddr> {
    pub epoch: Epoch,
    pub block_id: BlockId,
    pub block_height: NodeHeight,
    pub decision: QuorumDecision,
    pub signature: ValidatorSignature<TAddr>,
}

impl<TAddr> LastSentVote<TAddr> {
    pub fn get<TTx: StateStoreReadTransaction<Addr = TAddr> + ?Sized>(tx: &mut TTx) -> Result<Self, StorageError> {
        tx.last_sent_vote_get()
    }

    pub fn set<TTx: StateStoreWriteTransaction<Addr = TAddr>>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.last_sent_vote_set(self)
    }
}

impl<TAddr> std::fmt::Display for LastSentVote<TAddr> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(block_id: {}, height: {}, {:?})",
            self.block_id, self.block_height, self.decision
        )
    }
}
