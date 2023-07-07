//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{cmp::Ordering, collections::BTreeMap};

use serde::{Deserialize, Serialize};
use tari_dan_common_types::ShardId;

use crate::consensus_models::{Decision, QcId, TransactionId};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct Evidence {
    evidence: BTreeMap<ShardId, Vec<QcId>>,
}

impl Evidence {
    pub const fn empty() -> Self {
        Self {
            evidence: BTreeMap::new(),
        }
    }

    pub fn all_shards_complete(&self) -> bool {
        self.evidence.values().all(|qc_ids| !qc_ids.is_empty())
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&ShardId, &mut Vec<QcId>)> {
        self.evidence.iter_mut()
    }
}

impl FromIterator<(ShardId, Vec<QcId>)> for Evidence {
    fn from_iter<T: IntoIterator<Item = (ShardId, Vec<QcId>)>>(iter: T) -> Self {
        Evidence {
            evidence: iter.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TransactionAtom {
    pub id: TransactionId,
    pub involved_shards: Vec<ShardId>,
    pub decision: Decision,
    pub evidence: Evidence,
    pub fee: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Command {
    /// Command to prepare a transaction.
    Prepare(TransactionAtom),
    LocalPrepared(TransactionAtom),
    Accept(TransactionAtom),
}

impl Command {
    pub fn transaction_id(&self) -> &TransactionId {
        match self {
            Command::Prepare(tx) => &tx.id,
            Command::LocalPrepared(tx) => &tx.id,
            Command::Accept(tx) => &tx.id,
        }
    }

    pub fn local_prepared(&self) -> Option<&TransactionAtom> {
        match self {
            Command::LocalPrepared(tx) => Some(tx),
            _ => None,
        }
    }
}

impl PartialOrd for Command {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.transaction_id().partial_cmp(other.transaction_id())
    }
}

impl Ord for Command {
    fn cmp(&self, other: &Self) -> Ordering {
        self.transaction_id().cmp(other.transaction_id())
    }
}
