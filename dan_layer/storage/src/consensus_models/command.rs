//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    cmp::Ordering,
    collections::HashMap,
    fmt::{Display, Formatter},
};

use indexmap::{IndexMap, IndexSet};
use serde::{Deserialize, Serialize};
use tari_dan_common_types::SubstateAddress;
use tari_transaction::{TransactionId, VersionedSubstateId};
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::{
    ExecutedTransaction,
    ForeignProposal,
    LeaderFee,
    SubstateLockFlag,
    TransactionRecord,
    VersionedSubstateIdLockIntent,
};
use crate::{
    consensus_models::{Decision, QcId},
    StateStoreReadTransaction,
    StorageError,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct Evidence {
    evidence: IndexMap<SubstateAddress, ShardEvidence>,
}

impl Evidence {
    pub fn empty() -> Self {
        Self {
            evidence: IndexMap::new(),
        }
    }

    pub fn from_inputs_and_outputs(
        transaction_id: TransactionId,
        resolved_inputs: &IndexSet<VersionedSubstateIdLockIntent>,
        resulting_outputs: &[VersionedSubstateId],
    ) -> Self {
        let mut deduped_evidence = HashMap::new();
        deduped_evidence.extend(resolved_inputs.iter().map(|input| {
            (input.to_substate_address(), ShardEvidence {
                qc_ids: IndexSet::new(),
                lock: input.lock_flag(),
            })
        }));

        let tx_reciept_address = SubstateAddress::for_transaction_receipt(transaction_id.into_receipt_address());
        deduped_evidence.extend(
            resulting_outputs
                .iter()
                .map(|output| output.to_substate_address())
                // Exclude transaction receipt address from evidence since all involved shards will commit it
                .filter(|output| *output != tx_reciept_address)
                .map(|output| {
                    (output, ShardEvidence {
                        qc_ids: IndexSet::new(),
                        lock: SubstateLockFlag::Write,
                    })
                }),
        );

        deduped_evidence.into_iter().collect()
    }

    pub fn all_shards_justified(&self) -> bool {
        // TODO: we should check that remote has one QC and local has three
        self.evidence.values().all(|qc_ids| !qc_ids.is_empty())
    }

    pub fn is_empty(&self) -> bool {
        self.evidence.is_empty()
    }

    pub fn len(&self) -> usize {
        self.evidence.len()
    }

    pub fn get(&self, substate_address: &SubstateAddress) -> Option<&ShardEvidence> {
        self.evidence.get(substate_address)
    }

    pub fn num_justified_shards(&self) -> usize {
        self.evidence
            .values()
            .filter(|evidence| !evidence.qc_ids.is_empty())
            .count()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&SubstateAddress, &ShardEvidence)> {
        self.evidence.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&SubstateAddress, &mut ShardEvidence)> {
        self.evidence.iter_mut()
    }

    pub fn substate_addresses_iter(&self) -> impl Iterator<Item = &SubstateAddress> + '_ {
        self.evidence.keys()
    }

    pub fn qc_ids_iter(&self) -> impl Iterator<Item = &QcId> + '_ {
        self.evidence.values().flat_map(|e| e.qc_ids.iter())
    }

    pub fn merge(&mut self, other: Evidence) -> &mut Self {
        for (substate_address, shard_evidence) in other.evidence {
            let entry = self.evidence.entry(substate_address).or_insert_with(|| ShardEvidence {
                qc_ids: IndexSet::new(),
                lock: shard_evidence.lock,
            });
            entry.qc_ids.extend(shard_evidence.qc_ids);
        }
        self
    }
}

impl FromIterator<(SubstateAddress, ShardEvidence)> for Evidence {
    fn from_iter<T: IntoIterator<Item = (SubstateAddress, ShardEvidence)>>(iter: T) -> Self {
        let mut evidence = iter.into_iter().collect::<IndexMap<_, _>>();
        evidence.sort_keys();
        Evidence { evidence }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ShardEvidence {
    #[cfg_attr(feature = "ts", ts(type = "Array<string>"))]
    pub qc_ids: IndexSet<QcId>,
    pub lock: SubstateLockFlag,
}

impl ShardEvidence {
    pub fn new(qc_ids: IndexSet<QcId>, lock: SubstateLockFlag) -> Self {
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
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct TransactionAtom {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub id: TransactionId,
    pub decision: Decision,
    pub evidence: Evidence,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub transaction_fee: u64,
    pub leader_fee: Option<LeaderFee>,
}

impl TransactionAtom {
    pub fn deferred(transaction_id: TransactionId) -> Self {
        Self {
            id: transaction_id,
            decision: Decision::Deferred,
            evidence: Evidence::empty(),
            transaction_fee: 0,
            leader_fee: None,
        }
    }

    pub fn id(&self) -> &TransactionId {
        &self.id
    }

    pub fn get_transaction<TTx: StateStoreReadTransaction>(&self, tx: &TTx) -> Result<TransactionRecord, StorageError> {
        TransactionRecord::get(tx, &self.id)
    }

    pub fn get_executed_transaction<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
    ) -> Result<ExecutedTransaction, StorageError> {
        ExecutedTransaction::get(tx, &self.id)
    }

    pub fn abort(self) -> Self {
        Self {
            decision: Decision::Abort,
            leader_fee: None,
            ..self
        }
    }
}

impl Display for TransactionAtom {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TransactionAtom({}, {}, {}, {})",
            self.id,
            self.decision,
            self.transaction_fee,
            self.leader_fee
                .as_ref()
                .map(|f| f.to_string())
                .unwrap_or_else(|| "--".to_string())
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub enum Command {
    /// Command to prepare a transaction.
    Prepare(TransactionAtom),
    LocalPrepared(TransactionAtom),
    Accept(TransactionAtom),
    ForeignProposal(ForeignProposal),
    LocalOnly(TransactionAtom),
    EndEpoch,
}

#[derive(PartialEq, Eq, Ord, PartialOrd)]
pub enum CommandId {
    TransactionId(TransactionId),
    ForeignProposal(ForeignProposal),
    EndEpoch,
}

impl Display for CommandId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandId::TransactionId(id) => write!(f, "Transaction({})", id),
            CommandId::ForeignProposal(fp) => write!(f, "ForeignProposal({})", fp.block_id),
            CommandId::EndEpoch => write!(f, "EndEpoch"),
        }
    }
}

impl Command {
    pub fn transaction(&self) -> Option<&TransactionAtom> {
        match self {
            Command::Prepare(tx) => Some(tx),
            Command::LocalPrepared(tx) => Some(tx),
            Command::Accept(tx) => Some(tx),
            Command::LocalOnly(tx) => Some(tx),
            Command::ForeignProposal(_) => None,
            Command::EndEpoch => None,
        }
    }

    pub fn id(&self) -> CommandId {
        match self {
            Command::Prepare(tx) => CommandId::TransactionId(tx.id),
            Command::LocalPrepared(tx) => CommandId::TransactionId(tx.id),
            Command::Accept(tx) => CommandId::TransactionId(tx.id),
            Command::LocalOnly(tx) => CommandId::TransactionId(tx.id),
            Command::ForeignProposal(foreign_proposal) => CommandId::ForeignProposal(foreign_proposal.clone()),
            Command::EndEpoch => CommandId::EndEpoch,
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

    pub fn local_only(&self) -> Option<&TransactionAtom> {
        match self {
            Command::LocalOnly(tx) => Some(tx),
            _ => None,
        }
    }

    pub fn committing(&self) -> Option<&TransactionAtom> {
        let committing = match self {
            Command::Accept(tx) => Some(tx),
            Command::LocalOnly(tx) => Some(tx),
            _ => None,
        };

        committing.filter(|t| t.decision.is_commit())
    }

    pub fn is_epoch_end(&self) -> bool {
        matches!(self, Command::EndEpoch)
    }

    pub fn involved_shards(&self) -> impl Iterator<Item = &SubstateAddress> + '_ {
        match self {
            Command::Prepare(tx) => tx.evidence.substate_addresses_iter(),
            Command::LocalPrepared(tx) => tx.evidence.substate_addresses_iter(),
            Command::Accept(tx) => tx.evidence.substate_addresses_iter(),
            Command::LocalOnly(tx) => tx.evidence.substate_addresses_iter(),
            Command::ForeignProposal(_) => panic!("ForeignProposal does not have involved shards"),
            Command::EndEpoch => panic!("EpochEvent does not have involved shards"),
        }
    }

    pub fn evidence(&self) -> &Evidence {
        match self {
            Command::Prepare(tx) => &tx.evidence,
            Command::LocalPrepared(tx) => &tx.evidence,
            Command::Accept(tx) => &tx.evidence,
            Command::LocalOnly(tx) => &tx.evidence,
            Command::ForeignProposal(_) => panic!("ForeignProposal does not have evidence"),
            Command::EndEpoch => panic!("EpochEvent does not have evidence"),
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
            Command::LocalOnly(tx) => write!(f, "LocalOnly({}, {})", tx.id, tx.decision),
            Command::ForeignProposal(fp) => write!(f, "ForeignProposal {}", fp.block_id),
            Command::EndEpoch => write!(f, "EndEpoch"),
        }
    }
}
