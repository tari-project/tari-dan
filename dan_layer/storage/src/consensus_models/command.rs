//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    cmp::Ordering,
    fmt::{Display, Formatter},
};

use serde::{Deserialize, Serialize};
use tari_common_types::types::PublicKey;
use tari_dan_common_types::ShardGroup;
use tari_engine_types::substate::SubstateId;
use tari_transaction::TransactionId;

use super::{
    AbortReason,
    BlockId,
    ExecutedTransaction,
    ForeignProposalAtom,
    LeaderFee,
    MintConfidentialOutputAtom,
    TransactionRecord,
};
use crate::{
    consensus_models::{evidence::Evidence, Decision},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
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

    pub fn abort(self, reason: AbortReason) -> Self {
        Self {
            decision: Decision::Abort(reason),
            leader_fee: None,
            ..self
        }
    }
}

impl Display for TransactionAtom {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TransactionAtom({}, {}, {}, ",
            self.id, self.decision, self.transaction_fee,
        )?;
        match self.leader_fee {
            Some(ref leader_fee) => write!(f, "{}", leader_fee)?,
            None => write!(f, "--")?,
        }
        write!(f, ")")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub enum Command {
    // Transaction Commands
    /// Request validators to prepare a local-only transaction
    LocalOnly(TransactionAtom),
    /// Request validators to prepare a transaction.
    Prepare(TransactionAtom),
    /// Request validators to agree that the transaction was prepared by all local validators.
    LocalPrepare(TransactionAtom),
    /// Request validators to agree that all involved shard groups prepared the transaction.
    AllPrepare(TransactionAtom),
    /// Request validators to agree that one or more involved shard groups did not prepare the transaction.
    SomePrepare(TransactionAtom),
    /// Request validators to accept (i.e. accept COMMIT/ABORT decision) a transaction. All foreign inputs are received
    /// and the transaction is executed with the same decision.
    LocalAccept(TransactionAtom),
    /// Request validators to agree that all involved shard groups agreed to ACCEPT the transaction.
    AllAccept(TransactionAtom),
    /// Request validators to agree that one or more involved shard groups did not agreed to ACCEPT the transaction.
    SomeAccept(TransactionAtom),
    // Validator node commands
    ForeignProposal(ForeignProposalAtom),
    MintConfidentialOutput(MintConfidentialOutputAtom),
    SuspendNode(SuspendNodeAtom),
    ResumeNode(ResumeNodeAtom),
    EndEpoch,
}

/// Defines the order in which commands should be processed in a block. "Smallest" comes first and "largest" comes last.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum CommandOrdering<'a> {
    ResumeNode,
    SuspendNode,
    /// Foreign proposals should come first in the block so that they are processed before commands
    ForeignProposal(ShardGroup, &'a BlockId),
    MintConfidentialOutput(&'a SubstateId),
    TransactionId(&'a TransactionId),
    EndEpoch,
}

impl Command {
    pub fn transaction(&self) -> Option<&TransactionAtom> {
        match self {
            Command::Prepare(tx) |
            Command::LocalPrepare(tx) |
            Command::AllPrepare(tx) |
            Command::SomePrepare(tx) |
            Command::LocalAccept(tx) |
            Command::AllAccept(tx) |
            Command::SomeAccept(tx) |
            Command::LocalOnly(tx) => Some(tx),
            Command::ForeignProposal(_) |
            Command::MintConfidentialOutput(_) |
            Command::SuspendNode(_) |
            Command::ResumeNode(_) |
            Command::EndEpoch => None,
        }
    }

    fn as_ordering(&self) -> CommandOrdering<'_> {
        match self {
            Command::Prepare(tx) |
            Command::LocalPrepare(tx) |
            Command::AllPrepare(tx) |
            Command::SomePrepare(tx) |
            Command::LocalAccept(tx) |
            Command::AllAccept(tx) |
            Command::SomeAccept(tx) |
            Command::LocalOnly(tx) => CommandOrdering::TransactionId(&tx.id),
            Command::ForeignProposal(foreign_proposal) => {
                // Order by shard group then by block id
                CommandOrdering::ForeignProposal(foreign_proposal.shard_group, &foreign_proposal.block_id)
            },
            Command::MintConfidentialOutput(mint) => CommandOrdering::MintConfidentialOutput(&mint.substate_id),
            Command::SuspendNode(_) => CommandOrdering::SuspendNode,
            Command::ResumeNode(_) => CommandOrdering::ResumeNode,
            Command::EndEpoch => CommandOrdering::EndEpoch,
        }
    }

    pub fn local_only(&self) -> Option<&TransactionAtom> {
        match self {
            Command::LocalOnly(tx) => Some(tx),
            _ => None,
        }
    }

    pub fn prepare(&self) -> Option<&TransactionAtom> {
        match self {
            Command::Prepare(tx) => Some(tx),
            _ => None,
        }
    }

    pub fn local_prepare(&self) -> Option<&TransactionAtom> {
        match self {
            Command::LocalPrepare(tx) => Some(tx),
            _ => None,
        }
    }

    pub fn some_prepare(&self) -> Option<&TransactionAtom> {
        match self {
            Command::SomePrepare(tx) => Some(tx),
            _ => None,
        }
    }

    pub fn local_accept(&self) -> Option<&TransactionAtom> {
        match self {
            Command::LocalAccept(tx) => Some(tx),
            _ => None,
        }
    }

    pub fn foreign_proposal(&self) -> Option<&ForeignProposalAtom> {
        match self {
            Command::ForeignProposal(tx) => Some(tx),
            _ => None,
        }
    }

    pub fn resume_node(&self) -> Option<&ResumeNodeAtom> {
        match self {
            Command::ResumeNode(atom) => Some(atom),
            _ => None,
        }
    }

    pub fn mint_confidential_output(&self) -> Option<&MintConfidentialOutputAtom> {
        match self {
            Command::MintConfidentialOutput(mint) => Some(mint),
            _ => None,
        }
    }

    pub fn all_accept(&self) -> Option<&TransactionAtom> {
        match self {
            Command::AllAccept(tx) => Some(tx),
            _ => None,
        }
    }

    pub fn some_accept(&self) -> Option<&TransactionAtom> {
        match self {
            Command::SomeAccept(tx) => Some(tx),
            _ => None,
        }
    }

    /// Returns Some if the command should result in finalising (COMMITing or ABORTing) the transaction, otherwise None.
    pub fn finalising(&self) -> Option<&TransactionAtom> {
        self.all_accept()
            .or_else(|| self.some_accept())
            .or_else(|| self.local_only())
    }

    /// Returns Some if the command should result in committing the transaction, otherwise None.
    pub fn committing(&self) -> Option<&TransactionAtom> {
        self.all_accept()
            .or_else(|| self.local_only())
            .filter(|t| t.decision.is_commit())
    }

    pub fn is_epoch_end(&self) -> bool {
        matches!(self, Command::EndEpoch)
    }

    pub fn is_mint_confidential_output(&self) -> bool {
        matches!(self, Command::MintConfidentialOutput(_))
    }

    pub fn is_local_prepare(&self) -> bool {
        matches!(self, Command::LocalPrepare(_))
    }

    pub fn is_local_accept(&self) -> bool {
        matches!(self, Command::LocalAccept(_))
    }
}

impl PartialOrd for Command {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Command {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_ordering().cmp(&other.as_ordering())
    }
}

impl Display for Command {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Command::LocalOnly(tx) => write!(f, "LocalOnly({}, {})", tx.id, tx.decision),
            Command::Prepare(tx) => write!(f, "Prepare({}, {})", tx.id, tx.decision),
            Command::LocalPrepare(tx) => write!(f, "LocalPrepare({}, {})", tx.id, tx.decision),
            Command::AllPrepare(tx) => write!(f, "AllPrepared({}, {})", tx.id, tx.decision),
            Command::SomePrepare(tx) => write!(f, "SomePrepared({}, {})", tx.id, tx.decision),
            Command::LocalAccept(tx) => write!(f, "LocalAccept({}, {})", tx.id, tx.decision),
            Command::AllAccept(tx) => write!(f, "AllAccept({}, {})", tx.id, tx.decision),
            Command::SomeAccept(tx) => write!(f, "SomeAccept({}, {})", tx.id, tx.decision),
            Command::ForeignProposal(fp) => write!(f, "ForeignProposal {}", fp.block_id),
            Command::MintConfidentialOutput(mint) => write!(f, "MintConfidentialOutput({})", mint.substate_id),
            Command::SuspendNode(atom) => write!(f, "SuspendNode({atom})"),
            Command::ResumeNode(atom) => write!(f, "ResumeNode({atom})"),
            Command::EndEpoch => write!(f, "EndEpoch"),
        }
    }
}

#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuspendNodeAtom {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_key: PublicKey,
}

impl Display for SuspendNodeAtom {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.public_key)
    }
}

#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeNodeAtom {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_key: PublicKey,
}

impl ResumeNodeAtom {
    pub fn delete_suspended_node<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.suspended_nodes_delete(&self.public_key)
    }
}

impl Display for ResumeNodeAtom {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.public_key)
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, str::FromStr};

    use super::*;

    #[test]
    fn ordering() {
        assert!(
            CommandOrdering::ForeignProposal(ShardGroup::new(32, 63), &BlockId::zero()) >
                CommandOrdering::ForeignProposal(ShardGroup::new(0, 31), &BlockId::zero())
        );
        assert!(
            CommandOrdering::ForeignProposal(ShardGroup::new(0, 64), &BlockId::zero()) <
                CommandOrdering::TransactionId(&TransactionId::default())
        );
        let substate_id =
            SubstateId::from_str("component_0000000000000000000000000000000000000000000000000000000000000000").unwrap();

        assert!(
            CommandOrdering::MintConfidentialOutput(&substate_id) <
                CommandOrdering::TransactionId(&TransactionId::default())
        );
        assert!(CommandOrdering::MintConfidentialOutput(&substate_id) < CommandOrdering::EndEpoch);
        let mut set = BTreeSet::new();
        let cmds = [
            Command::EndEpoch,
            Command::MintConfidentialOutput(MintConfidentialOutputAtom { substate_id }),
            Command::ForeignProposal(ForeignProposalAtom {
                block_id: BlockId::zero(),
                shard_group: ShardGroup::new(0, 64),
            }),
            Command::Prepare(TransactionAtom {
                id: TransactionId::default(),
                decision: Decision::Commit,
                evidence: Evidence::default(),
                transaction_fee: 0,
                leader_fee: None,
            }),
        ];
        let expected = [cmds[2].clone(), cmds[1].clone(), cmds[3].clone(), cmds[0].clone()];
        set.extend(cmds);

        // Check the ordering in the set
        let mut iter = set.iter();
        for exp in &expected {
            let next = iter.next().unwrap();
            assert_eq!(next, exp);
        }
    }
}
