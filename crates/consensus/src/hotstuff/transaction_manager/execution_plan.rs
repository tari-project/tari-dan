//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    cmp::Reverse,
    collections::{HashMap, hash_map::Entry},
};

use tari_engine_types::substate::SubstateId;

/// What a transaction declares it touches, and how heavily it weighs on a block.
#[derive(Debug, Clone)]
pub struct DeclaredAccess {
    pub weight: u64,
    /// Each substate the transaction touches, and whether it may write it.
    pub substates: Vec<(SubstateId, bool)>,
}

/// Transactions that must speculate one after another, because an earlier one may write what a later one touches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionUnit {
    /// Indexes into the transactions the plan was made for, in their order.
    members: Vec<usize>,
    weight: u64,
}

impl ExecutionUnit {
    pub fn members(&self) -> &[usize] {
        &self.members
    }

    pub fn weight(&self) -> u64 {
        self.weight
    }
}

/// A block's transactions split into units that may speculate in parallel. Two transactions share a unit when, by
/// what they declare, the earlier may write a substate the later touches. Transactions that only read a substate in
/// common stay apart, because neither can change what the other reads.
///
/// Units carry no consensus meaning. A transaction that touches a substate an earlier one creates without declaring it
/// is a hidden dependency: its speculation reads state the block will not have, so it misses and is executed again in
/// block order.
#[derive(Debug, Clone, Default)]
pub struct ExecutionPlan {
    /// Heaviest first, so that the longest unit starts first.
    units: Vec<ExecutionUnit>,
}

impl ExecutionPlan {
    /// Plans `transactions`, given in the order the block evaluates them.
    pub fn new(transactions: &[DeclaredAccess]) -> Self {
        let mut sets = DisjointSets::new(transactions.len());
        let mut last_writer = HashMap::<&SubstateId, usize>::new();
        for (idx, access) in transactions.iter().enumerate() {
            for (id, is_write) in &access.substates {
                if let Some(writer) = last_writer.get(id) {
                    sets.union(*writer, idx);
                }
                if *is_write {
                    last_writer.insert(id, idx);
                }
            }
        }

        let mut by_root = HashMap::<usize, ExecutionUnit>::new();
        let mut roots = Vec::new();
        for (idx, access) in transactions.iter().enumerate() {
            let root = sets.find(idx);
            let unit = match by_root.entry(root) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => {
                    roots.push(root);
                    entry.insert(ExecutionUnit {
                        members: Vec::new(),
                        weight: 0,
                    })
                },
            };
            unit.members.push(idx);
            unit.weight = unit.weight.saturating_add(access.weight);
        }

        let mut units = roots
            .into_iter()
            .map(|root| by_root.remove(&root).expect("every root has a unit"))
            .collect::<Vec<_>>();
        // Stable, so units of equal weight keep the order of their first transaction
        units.sort_by_key(|unit| Reverse(unit.weight));
        Self { units }
    }

    /// Heaviest first.
    pub fn units(&self) -> &[ExecutionUnit] {
        &self.units
    }
}

struct DisjointSets {
    parents: Vec<usize>,
}

impl DisjointSets {
    fn new(len: usize) -> Self {
        Self {
            parents: (0..len).collect(),
        }
    }

    fn find(&mut self, mut idx: usize) -> usize {
        while self.parents[idx] != idx {
            self.parents[idx] = self.parents[self.parents[idx]];
            idx = self.parents[idx];
        }
        idx
    }

    fn union(&mut self, a: usize, b: usize) {
        let a = self.find(a);
        let b = self.find(b);
        if a != b {
            self.parents[a.max(b)] = a.min(b);
        }
    }
}

#[cfg(test)]
mod tests {
    use tari_template_lib_types::{ComponentAddress, ObjectKey};

    use super::*;

    fn id(seed: u8) -> SubstateId {
        ComponentAddress::from_array([seed; ObjectKey::LENGTH]).into()
    }

    fn access(substates: &[(&SubstateId, bool)]) -> DeclaredAccess {
        DeclaredAccess {
            weight: 1,
            substates: substates
                .iter()
                .map(|(id, is_write)| ((*id).clone(), *is_write))
                .collect(),
        }
    }

    fn members(plan: &ExecutionPlan) -> Vec<Vec<usize>> {
        let mut members = plan.units().iter().map(|u| u.members().to_vec()).collect::<Vec<_>>();
        members.sort();
        members
    }

    #[test]
    fn transactions_that_only_read_a_substate_in_common_stay_apart() {
        let token = id(1);
        let plan = ExecutionPlan::new(&[
            access(&[(&token, false), (&id(2), true)]),
            access(&[(&token, false), (&id(3), true)]),
        ]);
        assert_eq!(members(&plan), vec![vec![0], vec![1]]);
    }

    #[test]
    fn a_write_joins_every_later_transaction_that_touches_the_substate() {
        let vault = id(1);
        let plan = ExecutionPlan::new(&[
            access(&[(&vault, true)]),
            access(&[(&id(2), true)]),
            access(&[(&vault, false)]),
            access(&[(&vault, true)]),
        ]);
        assert_eq!(members(&plan), vec![vec![0, 2, 3], vec![1]]);
    }

    #[test]
    fn a_read_before_a_write_stays_apart() {
        let shared = id(1);
        let plan = ExecutionPlan::new(&[access(&[(&shared, false)]), access(&[(&shared, true)])]);
        assert_eq!(members(&plan), vec![vec![0], vec![1]]);
    }

    #[test]
    fn units_join_transitively() {
        let (a, b) = (id(1), id(2));
        let plan = ExecutionPlan::new(&[
            access(&[(&a, true)]),
            access(&[(&b, true)]),
            access(&[(&a, false), (&b, false)]),
        ]);
        assert_eq!(members(&plan), vec![vec![0, 1, 2]]);
    }

    #[test]
    fn the_heaviest_unit_comes_first() {
        let vault = id(1);
        let plan = ExecutionPlan::new(&[
            access(&[(&id(2), true)]),
            access(&[(&vault, true)]),
            access(&[(&vault, true)]),
        ]);
        assert_eq!(plan.units()[0].members(), [1, 2]);
        assert_eq!(plan.units()[0].weight(), 2);
        assert_eq!(plan.units()[1].members(), [0]);
    }
}
