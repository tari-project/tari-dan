//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, fmt};

use tari_consensus_types::BlockId;
use tari_engine_types::substate::Substate;
use tari_ootle_common_types::{SubstateRequirement, SubstateVersion};
use tari_ootle_storage::consensus_models::{LockedEpoch, TransactionExecution};
use tari_ootle_transaction::TransactionId;

/// Executions speculated for one block before it is evaluated in order, each usable only by an evaluation that gives
/// the transaction the inputs it was speculated with.
///
/// The engine is a pure function of the transaction, its input substates, the locked epoch, the input fanout and the
/// templates of the chain the block builds on. The fanout and templates are the same for every transaction of the
/// block the memo was speculated for, so a lookup names that block, and compares the input versions and the locked
/// epoch.
///
/// An input version names one value along a chain, except a validator fee pool, which a block rewrites in place
/// without a new version when fees are withdrawn from it. An execution with a validator fee pool input is therefore
/// never recorded, so that equal versions always mean equal inputs.
#[derive(Debug)]
pub struct ExecutionMemo {
    parent_block: BlockId,
    entries: HashMap<TransactionId, SpeculatedExecution>,
    num_hits: usize,
    num_misses: usize,
}

#[derive(Debug)]
struct SpeculatedExecution {
    input_versions: HashMap<SubstateRequirement, SubstateVersion>,
    locked_epoch: LockedEpoch,
    execution: TransactionExecution,
}

impl ExecutionMemo {
    /// A memo for the block that builds on `parent_block`.
    pub fn new(parent_block: BlockId) -> Self {
        Self {
            parent_block,
            entries: HashMap::new(),
            num_hits: 0,
            num_misses: 0,
        }
    }

    /// A memo that holds no executions.
    pub fn empty() -> Self {
        Self::new(BlockId::zero())
    }

    /// Records the execution of `transaction_id` against `inputs` at `locked_epoch`. Returns false, recording nothing,
    /// if an input's version does not name its value.
    pub fn insert(
        &mut self,
        transaction_id: TransactionId,
        inputs: &HashMap<SubstateRequirement, Substate>,
        locked_epoch: LockedEpoch,
        execution: TransactionExecution,
    ) -> bool {
        if inputs.keys().any(|req| req.substate_id().is_validator_fee_pool()) {
            return false;
        }
        self.entries.insert(transaction_id, SpeculatedExecution {
            input_versions: inputs
                .iter()
                .map(|(req, substate)| (req.clone(), substate.version()))
                .collect(),
            locked_epoch,
            execution,
        });
        true
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
        let is_same_inputs = entry.input_versions.len() == inputs.len() &&
            inputs
                .iter()
                .all(|(req, substate)| entry.input_versions.get(req) == Some(&substate.version()));
        if is_same_inputs && entry.locked_epoch == *locked_epoch {
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

/// Hits, misses, and speculated executions no evaluation asked for.
impl fmt::Display for ExecutionMemo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
    use tari_ootle_common_types::Epoch;
    use tari_template_lib_types::{ComponentAddress, ObjectKey, ValidatorFeePoolAddress};

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
        assert!(memo.insert(TransactionId::default(), inputs, locked_epoch, execution));
        memo
    }

    #[test]
    fn the_same_inputs_and_locked_epoch_hit() {
        let mut memo = memo_with(&inputs_at(3), pinned_to(5));
        let hit = memo.take(
            &BlockId::zero(),
            &TransactionId::default(),
            &inputs_at(3),
            &pinned_to(5),
        );
        assert!(hit.is_some());
        assert_eq!(memo.num_hits(), 1);
        assert_eq!(memo.num_untaken(), 0);
    }

    #[test]
    fn a_different_input_version_misses() {
        let mut memo = memo_with(&inputs_at(3), pinned_to(5));
        let hit = memo.take(
            &BlockId::zero(),
            &TransactionId::default(),
            &inputs_at(4),
            &pinned_to(5),
        );
        assert!(hit.is_none());
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
        let hit = memo.take(&BlockId::zero(), &TransactionId::default(), &more, &pinned_to(5));
        assert!(hit.is_none());
    }

    #[test]
    fn a_different_locked_epoch_misses() {
        let mut memo = memo_with(&inputs_at(3), pinned_to(5));
        let hit = memo.take(
            &BlockId::zero(),
            &TransactionId::default(),
            &inputs_at(3),
            &pinned_to(4),
        );
        assert!(hit.is_none());
        assert_eq!(memo.num_misses(), 1);
    }

    #[test]
    fn a_lookup_for_another_block_misses() {
        let mut memo = memo_with(&inputs_at(3), pinned_to(5));
        let other = BlockId::from([1u8; 32]);
        let hit = memo.take(&other, &TransactionId::default(), &inputs_at(3), &pinned_to(5));
        assert!(hit.is_none());
    }

    #[test]
    fn an_execution_with_a_validator_fee_pool_input_is_not_recorded() {
        let mut memo = ExecutionMemo::new(BlockId::zero());
        let fee_pool = SubstateRequirement::unversioned(SubstateId::from(ValidatorFeePoolAddress::from_array(
            [7; ObjectKey::LENGTH],
        )));
        let inputs = HashMap::from([(
            fee_pool,
            Substate::new(SubstateVersion::ZERO, NonFungibleContainer::no_data()),
        )]);
        let execution = TransactionExecution::abort(
            &TransactionId::default(),
            RejectReason::execution_failure_unclassified("test"),
        );
        assert!(!memo.insert(TransactionId::default(), &inputs, pinned_to(5), execution));
        assert_eq!(memo.num_untaken(), 0);
    }
}
