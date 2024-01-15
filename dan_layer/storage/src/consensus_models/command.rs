//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    cmp::Ordering,
    fmt::{Display, Formatter},
};

use indexmap::{IndexMap, IndexSet};
use serde::{Deserialize, Serialize};
use tari_dan_common_types::ShardId;
use tari_engine_types::lock::LockFlag;
use tari_transaction::TransactionId;

use super::ForeignProposal;
use crate::{
    consensus_models::{Decision, ExecutedTransaction, QcId},
    StateStoreReadTransaction,
    StorageError,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Evidence {
    evidence: IndexMap<ShardId, ShardEvidence>,
}

impl Evidence {
    pub fn empty() -> Self {
        Self {
            evidence: IndexMap::new(),
        }
    }

    pub fn all_shards_complete(&self) -> bool {
        // TODO: we should check that remote has one QC and local has three
        self.evidence.values().all(|qc_ids| !qc_ids.is_empty())
    }

    pub fn is_empty(&self) -> bool {
        self.evidence.is_empty()
    }

    pub fn len(&self) -> usize {
        self.evidence.len()
    }

    pub fn get(&self, shard_id: &ShardId) -> Option<&ShardEvidence> {
        self.evidence.get(shard_id)
    }

    pub fn num_complete_shards(&self) -> usize {
        self.evidence
            .values()
            .filter(|evidence| !evidence.qc_ids.is_empty())
            .count()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&ShardId, &ShardEvidence)> {
        self.evidence.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&ShardId, &mut ShardEvidence)> {
        self.evidence.iter_mut()
    }

    pub fn shards_iter(&self) -> impl Iterator<Item = &ShardId> + '_ {
        self.evidence.keys()
    }

    pub fn qc_ids_iter(&self) -> impl Iterator<Item = &QcId> + '_ {
        self.evidence.values().flat_map(|e| e.qc_ids.iter())
    }

    pub fn merge(&mut self, other: Evidence) -> &mut Self {
        for (shard_id, shard_evidence) in other.evidence {
            let entry = self.evidence.entry(shard_id).or_insert_with(|| ShardEvidence {
                qc_ids: IndexSet::new(),
                lock: shard_evidence.lock,
            });
            entry.qc_ids.extend(shard_evidence.qc_ids);
        }
        self
    }
}

impl FromIterator<(ShardId, ShardEvidence)> for Evidence {
    fn from_iter<T: IntoIterator<Item = (ShardId, ShardEvidence)>>(iter: T) -> Self {
        Evidence {
            evidence: iter.into_iter().collect(),
        }
    }
}

impl Extend<(ShardId, ShardEvidence)> for Evidence {
    fn extend<T: IntoIterator<Item = (ShardId, ShardEvidence)>>(&mut self, iter: T) {
        self.evidence.extend(iter.into_iter())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardEvidence {
    pub qc_ids: IndexSet<QcId>,
    pub lock: LockFlag,
}

impl ShardEvidence {
    pub fn new(qc_ids: IndexSet<QcId>, lock: LockFlag) -> Self {
        Self { qc_ids, lock }
    }

    pub fn is_empty(&self) -> bool {
        self.qc_ids.is_empty()
    }

    pub fn contains(&self, qc_id: &QcId) -> bool {
        self.qc_ids.contains(qc_id)
    }

    pub fn insert(&mut self, qc_id: QcId) {
        self.qc_ids.insert(qc_id);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl Display for TransactionAtom {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TransactionAtom({}, {}, {}, {})",
            self.id, self.decision, self.transaction_fee, self.leader_fee
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    /// Command to prepare a transaction.
    Prepare(TransactionAtom),
    LocalPrepared(TransactionAtom),
    Accept(TransactionAtom),
    ForeignProposal(ForeignProposal),
}

#[derive(PartialEq, Eq, Ord, PartialOrd)]
pub enum CommandId {
    TransactionId(TransactionId),
    ForeignProposal(ForeignProposal),
}

impl Display for CommandId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandId::TransactionId(id) => write!(f, "Transaction({})", id),
            CommandId::ForeignProposal(fp) => write!(f, "ForeignProposal({})", fp.block_id),
        }
    }
}

impl Command {
    pub fn transaction(&self) -> Option<&TransactionAtom> {
        match self {
            Command::Prepare(tx) => Some(tx),
            Command::LocalPrepared(tx) => Some(tx),
            Command::Accept(tx) => Some(tx),
            Command::ForeignProposal(_) => None,
        }
    }

    pub fn id(&self) -> CommandId {
        match self {
            Command::Prepare(tx) => CommandId::TransactionId(tx.id),
            Command::LocalPrepared(tx) => CommandId::TransactionId(tx.id),
            Command::Accept(tx) => CommandId::TransactionId(tx.id),
            Command::ForeignProposal(foreign_proposal) => CommandId::ForeignProposal(foreign_proposal.clone()),
        }
    }

    pub fn decision(&self) -> Decision {
        match self {
            Command::Prepare(tx) => tx.decision,
            Command::LocalPrepared(tx) => tx.decision,
            Command::Accept(tx) => tx.decision,
            Command::ForeignProposal(_) => panic!("ForeignProposal does not have a decision"),
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

    pub fn foreign_proposal(&self) -> Option<&ForeignProposal> {
        match self {
            Command::ForeignProposal(tx) => Some(tx),
            _ => None,
        }
    }

    pub fn involved_shards(&self) -> impl Iterator<Item = &ShardId> + '_ {
        match self {
            Command::Prepare(tx) => tx.evidence.shards_iter(),
            Command::LocalPrepared(tx) => tx.evidence.shards_iter(),
            Command::Accept(tx) => tx.evidence.shards_iter(),
            Command::ForeignProposal(_) => panic!("ForeignProposal does not have involved shards"),
        }
    }

    pub fn evidence(&self) -> &Evidence {
        match self {
            Command::Prepare(tx) => &tx.evidence,
            Command::LocalPrepared(tx) => &tx.evidence,
            Command::Accept(tx) => &tx.evidence,
            Command::ForeignProposal(_) => panic!("ForeignProposal does not have evidence"),
        }
    }
}

impl PartialOrd for Command {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Command {
    fn cmp(&self, other: &Self) -> Ordering {
        self.id().cmp(&other.id())
    }
}

impl Display for Command {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Command::Prepare(tx) => write!(f, "Prepare({}, {})", tx.id, tx.decision),
            Command::LocalPrepared(tx) => write!(f, "LocalPrepared({}, {})", tx.id, tx.decision),
            Command::Accept(tx) => write!(f, "Accept({}, {})", tx.id, tx.decision),
            Command::ForeignProposal(fp) => write!(f, "ForeignProposal {}", fp.block_id),
        }
    }
}
