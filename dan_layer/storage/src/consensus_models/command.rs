//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    cmp::Ordering,
    collections::BTreeMap,
    fmt::{Display, Formatter},
};

use serde::{Deserialize, Serialize};
use tari_dan_common_types::ShardId;
use tari_transaction::TransactionId;

use crate::{
    consensus_models::{Decision, ExecutedTransaction, QcId},
    StateStoreReadTransaction,
    StorageError,
};

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

    pub fn is_empty(&self) -> bool {
        self.evidence.is_empty()
    }

    pub fn len(&self) -> usize {
        self.evidence.len()
    }

    pub fn num_complete_shards(&self) -> usize {
        self.evidence.values().filter(|qc_ids| !qc_ids.is_empty()).count()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&ShardId, &Vec<QcId>)> {
        self.evidence.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&ShardId, &mut Vec<QcId>)> {
        self.evidence.iter_mut()
    }

    pub fn shards_iter(&self) -> impl Iterator<Item = &ShardId> + '_ {
        self.evidence.keys()
    }

    pub fn qc_ids_iter(&self) -> impl Iterator<Item = &QcId> + '_ {
        self.evidence.values().flatten()
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
    pub decision: Decision,
    pub evidence: Evidence,
    pub transaction_fee: u64,
    pub leader_fee: u64,
}

impl TransactionAtom {
    pub fn id(&self) -> &TransactionId {
        &self.id
    }

    pub fn get_transaction<TTx: StateStoreReadTransaction>(
        &self,
        tx: &mut TTx,
    ) -> Result<ExecutedTransaction, StorageError> {
        ExecutedTransaction::get(tx, &self.id)
    }
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

    pub fn decision(&self) -> Decision {
        match self {
            Command::Prepare(tx) => tx.decision,
            Command::LocalPrepared(tx) => tx.decision,
            Command::Accept(tx) => tx.decision,
        }
    }

    pub fn prepare(&self) -> Option<&TransactionAtom> {
        match self {
            Command::Prepare(tx) => Some(tx),
            _ => None,
        }
    }

    pub fn local_prepared(&self) -> Option<&TransactionAtom> {
        match self {
            Command::LocalPrepared(tx) => Some(tx),
            _ => None,
        }
    }

    pub fn accept(&self) -> Option<&TransactionAtom> {
        match self {
            Command::Accept(tx) => Some(tx),
            _ => None,
        }
    }

    pub fn involved_shards(&self) -> impl Iterator<Item = &ShardId> + '_ {
        match self {
            Command::Prepare(tx) => tx.evidence.shards_iter(),
            Command::LocalPrepared(tx) => tx.evidence.shards_iter(),
            Command::Accept(tx) => tx.evidence.shards_iter(),
        }
    }

    pub fn evidence(&self) -> &Evidence {
        match self {
            Command::Prepare(tx) => &tx.evidence,
            Command::LocalPrepared(tx) => &tx.evidence,
            Command::Accept(tx) => &tx.evidence,
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

impl Display for Command {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Command::Prepare(tx) => write!(f, "Prepare({}, {})", tx.id, tx.decision),
            Command::LocalPrepared(tx) => write!(f, "LocalPrepared({}, {})", tx.id, tx.decision),
            Command::Accept(tx) => write!(f, "Accept({}, {})", tx.id, tx.decision),
        }
    }
}
