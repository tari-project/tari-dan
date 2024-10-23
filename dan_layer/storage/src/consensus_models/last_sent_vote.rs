//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::{Epoch, NodeHeight};

use super::{QuorumDecision, ValidatorSignature};
use crate::{consensus_models::BlockId, StateStoreReadTransaction, StateStoreWriteTransaction, StorageError};

pub struct LastSentVote {
    pub epoch: Epoch,
    pub block_id: BlockId,
    pub block_height: NodeHeight,
    pub decision: QuorumDecision,
    pub signature: ValidatorSignature,
}

impl LastSentVote {
    pub fn get<TTx: StateStoreReadTransaction>(tx: &TTx) -> Result<Self, StorageError> {
        tx.last_sent_vote_get()
    }

    pub fn set<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.last_sent_vote_set(self)
    }
}

impl std::fmt::Display for LastSentVote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(block_id: {}, height: {}, {:?})",
            self.block_id, self.block_height, self.decision
        )
    }
}
