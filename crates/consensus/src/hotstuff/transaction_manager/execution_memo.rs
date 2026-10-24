//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, fmt, time::Duration};

use tari_consensus_types::BlockId;
use tari_engine_types::substate::Substate;
use tari_ootle_common_types::SubstateRequirement;
use tari_ootle_storage::consensus_models::{LockedEpoch, TransactionExecution};
use tari_ootle_transaction::TransactionId;

/// Executions speculated for one block before it is evaluated in order, each usable only by an evaluation that gives
/// the transaction the inputs it was speculated with.
///
/// The engine is a pure function of the transaction, its input substates, the locked epoch, the input fanout and the
/// templates of the chain the block builds on. The fanout and templates are the same for every transaction of the
/// block the memo was speculated for, so a lookup names that block, and compares the input substates, value and
/// version, and the locked epoch.
///
/// A version alone does not name a value here. Speculation applies changes the block may never make, so two
/// speculated transactions can create the same substate at the same version with different values, and a block
/// rewrites a validator fee pool in place when fees are withdrawn from it.
#[derive(Debug)]
pub struct ExecutionMemo {
    parent_block: BlockId,
    entries: HashMap<TransactionId, SpeculatedExecution>,
    summary: Option<SpeculationSummary>,
    num_hits: usize,
    num_misses: usize,
}

#[derive(Debug)]
struct SpeculatedExecution {
    inputs: SpeculatedInputs,
    locked_epoch: LockedEpoch,
    execution: TransactionExecution,
}

/// The substates an execution read, each encoded with its version and value, by the requirement it resolved.
#[derive(Debug)]
pub struct SpeculatedInputs(HashMap<SubstateRequirement, Vec<u8>>);

impl SpeculatedInputs {
    /// The encoding of `inputs`, or `None` if one cannot be encoded.
    pub fn encode(inputs: &HashMap<SubstateRequirement, Substate>) -> Option<Self> {
        inputs
            .iter()
            .map(|(req, substate)| Some((req.clone(), tari_bor::encode(substate).ok()?)))
            .collect::<Option<_>>()
            .map(Self)
    }

    /// Whether `inputs` are exactly these. Equal substates encode alike, so a substate whose encoding differs is a
    /// different substate.
    fn matches(&self, inputs: &HashMap<SubstateRequirement, Substate>) -> bool {
        self.0.len() == inputs.len() &&
            inputs.iter().all(|(req, substate)| {
                self.0
                    .get(req)
                    .is_some_and(|encoded| tari_bor::encode(substate).is_ok_and(|bytes| bytes == *encoded))
            })
    }
}

/// How a block's transactions were speculated.
#[derive(Debug, Clone, Copy, Default)]
pub struct SpeculationSummary {
    pub num_candidates: usize,
    pub num_speculated: usize,
    pub num_units: usize,
    /// The number of transactions in the unit with the most, and their summed weight.
    pub longest_unit: (usize, u64),
    pub num_threads: usize,
    pub elapsed: Duration,
    /// Whether speculation stopped at its time or execution budget before every unit was done.
    pub is_budget_spent: bool,
}

impl ExecutionMemo {
    /// A memo for the block that builds on `parent_block`.
    pub fn new(parent_block: BlockId) -> Self {
        Self {
            parent_block,
            entries: HashMap::new(),
            summary: None,
            num_hits: 0,
            num_misses: 0,
        }
    }

    /// A memo that holds no executions.
    pub fn empty() -> Self {
        Self::new(BlockId::zero())
    }

    /// Records the execution of `transaction_id` against `inputs` at `locked_epoch`.
    pub fn insert(
        &mut self,
        transaction_id: TransactionId,
        inputs: SpeculatedInputs,
        locked_epoch: LockedEpoch,
        execution: TransactionExecution,
    ) {
        self.entries.insert(transaction_id, SpeculatedExecution {
            inputs,
            locked_epoch,
            execution,
        });
    }

    pub fn set_summary(&mut self, summary: SpeculationSummary) {
        self.summary = Some(summary);
    }

    /// Whether the block's transactions were speculated.
    pub fn is_speculated(&self) -> bool {
        self.summary.is_some()
    }

    /// The execution speculated for `transaction_id` in the block that builds on `parent_block`, if it was speculated
    /// with exactly `inputs` at `locked_epoch`. An execution is taken at most once.
    pub fn take(
        &mut self,
        parent_block: &BlockId,
        transaction_id: &TransactionId,
        inputs: &HashMap<SubstateRequirement, Substate>,
        locked_epoch: &LockedEpoch,
    ) -> Option<TransactionExecution> {
        if *parent_block != self.parent_block {
            return None;
        }
        let entry = self.entries.remove(transaction_id)?;
        if entry.locked_epoch == *locked_epoch && entry.inputs.matches(inputs) {
            self.num_hits += 1;
            Some(entry.execution)
        } else {
            self.num_misses += 1;
            None
        }
    }

    /// The executions not yet taken.
    pub fn num_untaken(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub fn num_hits(&self) -> usize {
        self.num_hits
    }

    #[cfg(test)]
    pub fn num_misses(&self) -> usize {
        self.num_misses
    }
}

impl fmt::Display for ExecutionMemo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(summary) = &self.summary {
            write!(
                f,
                "speculated {} of {} transaction(s) in {} unit(s) on {} thread(s) in {:.2?}{}, longest unit {} \
                 transaction(s) of weight {}; ",
                summary.num_speculated,
                summary.num_candidates,
                summary.num_units,
                summary.num_threads,
                summary.elapsed,
                if summary.is_budget_spent { " (budget spent)" } else { "" },
                summary.longest_unit.0,
                summary.longest_unit.1,
            )?;
        }
        write!(
            f,
            "{} hit(s), {} miss(es), {} unused",
            self.num_hits,
            self.num_misses,
            self.entries.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use tari_engine_types::{commit_result::RejectReason, non_fungible::NonFungibleContainer, substate::SubstateId};
    use tari_ootle_common_types::{Epoch, SubstateVersion};
    use tari_template_lib_types::{ComponentAddress, ObjectKey};

    use super::*;

    fn requirement(seed: u8) -> SubstateRequirement {
        SubstateRequirement::unversioned(SubstateId::from(ComponentAddress::from_array(
            [seed; ObjectKey::LENGTH],
        )))
    }

    fn inputs_at(version: u64) -> HashMap<SubstateRequirement, Substate> {
        HashMap::from([(
            requirement(1),
            Substate::new(SubstateVersion::new(version), NonFungibleContainer::no_data()),
        )])
    }

    fn pinned_to(epoch: u64) -> LockedEpoch {
        LockedEpoch::new(Epoch(epoch), Default::default(), Default::default())
    }

    fn memo_with(inputs: &HashMap<SubstateRequirement, Substate>, locked_epoch: LockedEpoch) -> ExecutionMemo {
        let mut memo = ExecutionMemo::new(BlockId::zero());
        let execution = TransactionExecution::abort(
            &TransactionId::default(),
            RejectReason::execution_failure_unclassified("test"),
        );
        memo.insert(
            TransactionId::default(),
            SpeculatedInputs::encode(inputs).unwrap(),
            locked_epoch,
            execution,
        );
        memo
    }

    fn take(
        memo: &mut ExecutionMemo,
        inputs: &HashMap<SubstateRequirement, Substate>,
        locked_epoch: LockedEpoch,
    ) -> Option<TransactionExecution> {
        memo.take(&BlockId::zero(), &TransactionId::default(), inputs, &locked_epoch)
    }

    #[test]
    fn the_same_inputs_and_locked_epoch_hit() {
        let mut memo = memo_with(&inputs_at(3), pinned_to(5));
        assert!(take(&mut memo, &inputs_at(3), pinned_to(5)).is_some());
        assert_eq!(memo.num_hits(), 1);
        assert_eq!(memo.num_untaken(), 0);
    }

    #[test]
    fn a_different_input_version_misses() {
        let mut memo = memo_with(&inputs_at(3), pinned_to(5));
        assert!(take(&mut memo, &inputs_at(4), pinned_to(5)).is_none());
        assert_eq!(memo.num_misses(), 1);
    }

    #[test]
    fn a_different_input_value_at_the_same_version_misses() {
        let mut memo = memo_with(&inputs_at(3), pinned_to(5));
        let mut burnt = NonFungibleContainer::no_data();
        burnt.burn();
        let other_value = HashMap::from([(requirement(1), Substate::new(SubstateVersion::new(3), burnt))]);
        assert!(take(&mut memo, &other_value, pinned_to(5)).is_none());
        assert_eq!(memo.num_misses(), 1);
    }

    #[test]
    fn a_different_input_set_misses() {
        let mut memo = memo_with(&inputs_at(3), pinned_to(5));
        let mut more = inputs_at(3);
        more.insert(
            requirement(2),
            Substate::new(SubstateVersion::ZERO, NonFungibleContainer::no_data()),
        );
        assert!(take(&mut memo, &more, pinned_to(5)).is_none());
    }

    #[test]
    fn a_different_locked_epoch_misses() {
        let mut memo = memo_with(&inputs_at(3), pinned_to(5));
        assert!(take(&mut memo, &inputs_at(3), pinned_to(4)).is_none());
        assert_eq!(memo.num_misses(), 1);
    }

    #[test]
    fn a_lookup_for_another_block_misses() {
        let mut memo = memo_with(&inputs_at(3), pinned_to(5));
        let other = BlockId::from([1u8; 32]);
        let hit = memo.take(&other, &TransactionId::default(), &inputs_at(3), &pinned_to(5));
        assert!(hit.is_none());
    }
}
