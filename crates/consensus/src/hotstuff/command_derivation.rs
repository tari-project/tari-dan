//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Builds the transaction commands of a local block from this node's own pool record and execution. The proposer
//! emits what it derives. A replica derives the same from its own record and execution and votes only if the result
//! equals what was proposed, so the two disagree only where their inputs do.

use std::num::NonZeroU64;

use tari_engine_types::substate::SubstateDiff;
use tari_ootle_common_types::{ShardGroup, committee::CommitteeInfo};
use tari_ootle_storage::consensus_models::{
    InvalidEvidenceReason,
    LocalOnlyAtom,
    NoVoteReason,
    TransactionExecution,
    TransactionPoolRecord,
};

use crate::hotstuff::transaction_manager::LocalPreparedTransaction;

/// A transaction command derived for a block, with what including it changes.
#[derive(Debug)]
pub(super) struct DerivedCommand<TAtom> {
    pub atom: TAtom,
    /// The pool record as the command leaves it, before it moves to its next stage.
    pub pool_record: TransactionPoolRecord,
    pub execution: TransactionExecution,
    /// The leader fee the block pays for this command.
    pub leader_fee: u64,
    /// This shard group's portion of the exhaust burn the block accumulates for this command.
    pub exhaust_burn: u64,
}

impl<TAtom> DerivedCommand<TAtom> {
    /// The diff the block applies for this command, if it commits the transaction.
    pub fn committed_diff(&self) -> Option<&SubstateDiff> {
        if self.pool_record.current_decision().is_commit() {
            self.execution.result().finalize.any_accept()
        } else {
            None
        }
    }
}

/// Why a transaction prepared as local-only cannot be sequenced as a LocalOnly command. A local-only execution's
/// evidence names exactly the local shard group, so a proposer's own preparation never produces either.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(super) enum LocalOnlyRefusal {
    #[error("local-only evidence names {num_shard_groups} shard groups")]
    EvidenceSpansShardGroups { num_shard_groups: usize },
    #[error("local shard group {shard_group} is not in the local-only evidence")]
    LocalShardGroupNotInEvidence { shard_group: ShardGroup },
}

impl From<LocalOnlyRefusal> for NoVoteReason {
    fn from(refusal: LocalOnlyRefusal) -> Self {
        match refusal {
            LocalOnlyRefusal::EvidenceSpansShardGroups { .. } => NoVoteReason::LocalOnlyProposedForMultiShard,
            LocalOnlyRefusal::LocalShardGroupNotInEvidence { shard_group } => NoVoteReason::InvalidEvidence {
                reason: InvalidEvidenceReason::MissingInvolvedShardGroup { shard_group },
            },
        }
    }
}

/// Derives the LocalOnly command for a transaction at the New stage that prepared as local-only.
pub(super) fn derive_local_only_command(
    mut pool_record: TransactionPoolRecord,
    prepared: LocalPreparedTransaction,
    local_committee_info: &CommitteeInfo,
) -> Result<DerivedCommand<LocalOnlyAtom>, LocalOnlyRefusal> {
    let (LocalPreparedTransaction::Accept { execution, .. } | LocalPreparedTransaction::EarlyAbort { execution }) =
        prepared;
    pool_record
        .set_local_decision(execution.decision())
        .set_transaction_fee(execution.transaction_fee())
        .set_exhaust_burn(execution.exhaust_burn())
        .set_evidence(execution.to_evidence(
            local_committee_info.num_preshards(),
            local_committee_info.num_committees(),
        ));

    let (leader_fee, exhaust_burn) = if pool_record.current_decision().is_commit() {
        // A LocalOnly transaction involves exactly the local shard group, so its leader fee is calculated with one
        // involved shard group and it accounts for the entire exhaust burn.
        let num_shard_groups = pool_record.evidence().num_shard_groups();
        if num_shard_groups != 1 {
            return Err(LocalOnlyRefusal::EvidenceSpansShardGroups { num_shard_groups });
        }
        let leader_fee = pool_record.calculate_leader_fee(NonZeroU64::new(1).expect("1 > 0"));
        let shard_group = local_committee_info.shard_group();
        let exhaust_burn = pool_record
            .evidence()
            .exhaust_burn_portion(leader_fee.exhaust_burn(), shard_group)
            .ok_or(LocalOnlyRefusal::LocalShardGroupNotInEvidence { shard_group })?;
        let fee = leader_fee.fee();
        pool_record.set_leader_fee(leader_fee);
        (fee, exhaust_burn)
    } else {
        pool_record.no_leader_fee();
        (0, 0)
    };

    Ok(DerivedCommand {
        atom: pool_record.get_local_only_atom(),
        pool_record,
        execution,
        leader_fee,
        exhaust_burn,
    })
}

/// The reason a replica refuses `proposed` when it derived `derived` for the same transaction. Fields are compared in
/// a fixed order, so a proposal that differs in several is refused for the first.
pub(super) fn local_only_disagreement(derived: &LocalOnlyAtom, proposed: &LocalOnlyAtom) -> Option<NoVoteReason> {
    if derived.decision != proposed.decision {
        return Some(NoVoteReason::DecisionDisagreement {
            local: derived.decision,
            remote: proposed.decision,
        });
    }
    if derived.transaction_fee != proposed.transaction_fee {
        return Some(NoVoteReason::FeeDisagreement);
    }
    match (&derived.leader_fee, &proposed.leader_fee) {
        (Some(_), None) => Some(NoVoteReason::NoLeaderFee),
        (derived, proposed) if derived != proposed => Some(NoVoteReason::LeaderFeeDisagreement),
        _ => None,
    }
}
