//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Builds the transaction commands of a local block from this node's own pool record and execution. The proposer
//! emits what it derives. A replica derives the same from its own record and execution and votes only if the result
//! equals what was proposed, so the two disagree only where their inputs do.

use std::{fmt, fmt::Display, num::NonZeroU64};

use tari_consensus_types::Decision;
use tari_engine_types::substate::SubstateDiff;
use tari_ootle_common_types::{ShardGroup, committee::CommitteeInfo};
use tari_ootle_storage::consensus_models::{
    Command,
    InvalidEvidenceReason,
    LeaderFee,
    LocalOnlyAtom,
    MultiShardAtom,
    NoVoteReason,
    TransactionExecution,
    TransactionPoolRecord,
    TransactionPoolStage,
};

use crate::hotstuff::{
    error::HotStuffError,
    transaction_manager::{EvidenceOrExecution, LocalPreparedTransaction, MultiShardPreparedTransaction},
};

/// A transaction command derived for a block, with what including it changes.
#[derive(Debug)]
pub(super) struct DerivedCommand<TCommand = Command> {
    pub command: TCommand,
    /// The pool record as the command leaves it, before it moves to its next stage.
    pub pool_record: TransactionPoolRecord,
    /// The execution the command is derived from. A transaction that pledges its local inputs to foreign shard
    /// groups has none until it has every input.
    pub execution: Option<TransactionExecution>,
    /// The leader fee the block pays for this command.
    pub leader_fee: u64,
    /// This shard group's portion of the exhaust burn the block accumulates for this command.
    pub exhaust_burn: u64,
}

impl DerivedCommand<Command> {
    /// The diff the block applies for this command: a committing LocalOnly command's execution diff. Every other
    /// command's diff is applied when the transaction finalizes.
    pub fn committed_diff(&self) -> Option<&SubstateDiff> {
        self.command
            .local_only()
            .filter(|atom| atom.decision.is_commit())
            .and(self.execution.as_ref())
            .and_then(|execution| execution.result().finalize.any_accept())
    }
}

/// The two commands a transaction at the New stage that involves foreign shard groups can be sequenced with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MultiShardCommandKind {
    LocalPrepare,
    LocalAccept,
}

impl MultiShardCommandKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalPrepare => "LocalPrepare",
            Self::LocalAccept => "LocalAccept",
        }
    }

    /// The stage the command moves the transaction to.
    pub const fn next_stage(self) -> TransactionPoolStage {
        match self {
            Self::LocalPrepare => TransactionPoolStage::LocalPrepared,
            Self::LocalAccept => TransactionPoolStage::LocalAccepted,
        }
    }
}

impl Display for MultiShardCommandKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A LocalPrepare or LocalAccept command for a transaction at the New stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NewMultiShardCommand {
    pub kind: MultiShardCommandKind,
    pub atom: MultiShardAtom,
}

impl NewMultiShardCommand {
    pub fn into_command(self) -> Command {
        match self.kind {
            MultiShardCommandKind::LocalPrepare => Command::LocalPrepare(self.atom),
            MultiShardCommandKind::LocalAccept => Command::LocalAccept(self.atom),
        }
    }
}

impl Display for NewMultiShardCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({})", self.kind, self.atom)
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
) -> Result<DerivedCommand, LocalOnlyRefusal> {
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
        command: Command::LocalOnly(pool_record.get_local_only_atom()),
        pool_record,
        execution: Some(execution),
        leader_fee,
        exhaust_burn,
    })
}

/// Derives the command for a transaction at the New stage that prepared as involving foreign shard groups: LocalAccept
/// if it aborts or this shard group only creates its outputs, otherwise LocalPrepare. Neither adds to the block's
/// leader fee or exhaust burn.
pub(super) fn derive_new_multishard_command(
    mut pool_record: TransactionPoolRecord,
    prepared: MultiShardPreparedTransaction,
    local_committee_info: &CommitteeInfo,
) -> Result<DerivedCommand<NewMultiShardCommand>, HotStuffError> {
    let local_shard_group = local_committee_info.shard_group();
    let execution = match prepared.into_evidence_or_execution() {
        EvidenceOrExecution::Execution { execution } => {
            // CASE: All inputs are local and outputs are foreign (i.e. the transaction is executed), or all inputs are
            // foreign, this shard group is output-only and has received every pledge.
            pool_record.update_from_execution(
                local_committee_info.num_preshards(),
                local_committee_info.num_committees(),
                &execution,
            );

            // TODO: this is kinda hacky - we may not be involved in the transaction after ABORT execution, but this
            // would be invalid so we ensure that we are added to evidence. Ideally, we wouldn't sequence this
            // transaction at all - investigate.
            pool_record.evidence_mut().add_shard_group(local_shard_group);

            if execution.decision().is_commit() && !pool_record.evidence().has_inputs(local_shard_group) {
                let num_involved_shard_groups = pool_record.evidence().num_shard_groups();
                let involved = NonZeroU64::new(num_involved_shard_groups as u64)
                    .ok_or_else(|| HotStuffError::InvariantError("Number of involved shard groups is 0".to_string()))?;
                let leader_fee = pool_record.calculate_leader_fee(involved);
                pool_record.set_leader_fee(leader_fee);
            }
            Some(*execution)
        },
        EvidenceOrExecution::Evidence { evidence } => {
            // CASE: All local inputs were resolved. Consensus continues to gather the foreign inputs. Evidence the
            // record already holds from foreign proposals is kept, so the local inputs are merged into it.
            pool_record
                .set_local_decision(Decision::Commit)
                .merge_evidence(evidence);
            None
        },
    };

    let kind = if pool_record.current_decision().is_abort() ||
        pool_record.evidence().is_committee_output_only(local_shard_group)
    {
        // Neither an aborting transaction nor one whose foreign shard groups have prepared every input needs a
        // prepare phase here
        MultiShardCommandKind::LocalAccept
    } else {
        MultiShardCommandKind::LocalPrepare
    };

    Ok(DerivedCommand {
        command: NewMultiShardCommand {
            kind,
            atom: pool_record.get_current_multi_shard_atom(),
        },
        pool_record,
        execution,
        leader_fee: 0,
        exhaust_burn: 0,
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
    leader_fee_disagreement(derived.leader_fee.as_ref(), proposed.leader_fee.as_ref())
}

/// The reason a replica refuses a `proposed_kind` command for a transaction at the New stage when it derived
/// `derived`. Fields are compared in a fixed order, so a proposal that differs in several is refused for the first.
pub(super) fn new_multishard_disagreement(
    derived: &NewMultiShardCommand,
    proposed_kind: MultiShardCommandKind,
    proposed: &MultiShardAtom,
) -> Option<NoVoteReason> {
    let NewMultiShardCommand {
        kind: derived_kind,
        atom: derived,
    } = derived;
    if derived.decision != proposed.decision {
        return Some(NoVoteReason::DecisionDisagreement {
            local: derived.decision,
            remote: proposed.decision,
        });
    }
    if *derived_kind != proposed_kind {
        return Some(NoVoteReason::CommandDisagreement {
            transaction_id: proposed.id,
            local: derived_kind.as_str(),
            remote: proposed_kind.as_str(),
        });
    }
    if derived.transaction_fee != proposed.transaction_fee {
        return Some(NoVoteReason::FeeDisagreement);
    }
    if let Some(reason) = leader_fee_disagreement(derived.leader_fee.as_ref(), proposed.leader_fee.as_ref()) {
        return Some(reason);
    }
    if derived.evidence != proposed.evidence {
        return Some(NoVoteReason::InvalidEvidence {
            reason: InvalidEvidenceReason::MismatchedEvidence,
        });
    }
    None
}

fn leader_fee_disagreement(derived: Option<&LeaderFee>, proposed: Option<&LeaderFee>) -> Option<NoVoteReason> {
    match (derived, proposed) {
        (Some(_), None) => Some(NoVoteReason::NoLeaderFee),
        (derived, proposed) if derived != proposed => Some(NoVoteReason::LeaderFeeDisagreement),
        _ => None,
    }
}
