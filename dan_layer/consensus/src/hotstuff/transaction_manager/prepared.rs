//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use indexmap::IndexMap;
use tari_dan_common_types::{NumPreshards, SubstateRequirement, VersionedSubstateId};
use tari_dan_storage::consensus_models::{Decision, Evidence, TransactionExecution, VersionedSubstateIdLockIntent};

use crate::hotstuff::substate_store::LockStatus;

#[derive(Debug)]
pub enum PreparedTransaction {
    LocalOnly(LocalPreparedTransaction),
    MultiShard(MultiShardPreparedTransaction),
}

impl PreparedTransaction {
    pub fn new_local_accept(executed: TransactionExecution, lock_status: LockStatus) -> Self {
        Self::LocalOnly(LocalPreparedTransaction::Accept {
            execution: executed,
            lock_status,
        })
    }

    pub fn new_local_early_abort(execution: TransactionExecution) -> Self {
        Self::LocalOnly(LocalPreparedTransaction::EarlyAbort { execution })
    }

    pub fn lock_status(&self) -> &LockStatus {
        static DEFAULT_LOCK_STATUS: LockStatus = LockStatus::new();
        match self {
            Self::LocalOnly(LocalPreparedTransaction::Accept { lock_status, .. }) => lock_status,
            Self::LocalOnly(LocalPreparedTransaction::EarlyAbort { .. }) => &DEFAULT_LOCK_STATUS,
            Self::MultiShard(multishard) => &multishard.lock_status,
        }
    }

    pub fn into_lock_status(self) -> LockStatus {
        match self {
            Self::LocalOnly(LocalPreparedTransaction::Accept { lock_status, .. }) => lock_status,
            Self::LocalOnly(LocalPreparedTransaction::EarlyAbort { .. }) => LockStatus::new(),
            Self::MultiShard(multishard) => multishard.lock_status,
        }
    }

    pub fn new_multishard(
        execution: Option<TransactionExecution>,
        local_inputs: IndexMap<SubstateRequirement, u32>,
        foreign_inputs: HashSet<SubstateRequirement>,
        outputs: HashSet<VersionedSubstateId>,
        lock_status: LockStatus,
    ) -> Self {
        Self::MultiShard(MultiShardPreparedTransaction {
            execution,
            local_inputs,
            foreign_inputs,
            outputs,
            lock_status,
        })
    }
}

#[derive(Debug)]
pub enum LocalPreparedTransaction {
    Accept {
        execution: TransactionExecution,
        lock_status: LockStatus,
    },
    EarlyAbort {
        execution: TransactionExecution,
    },
}

#[derive(Debug)]
pub struct MultiShardPreparedTransaction {
    execution: Option<TransactionExecution>,
    local_inputs: IndexMap<SubstateRequirement, u32>,
    outputs: HashSet<VersionedSubstateId>,
    foreign_inputs: HashSet<SubstateRequirement>,
    lock_status: LockStatus,
}

impl MultiShardPreparedTransaction {
    pub fn is_executed(&self) -> bool {
        self.execution.is_some()
    }

    pub fn current_decision(&self) -> Decision {
        self.execution
            .as_ref()
            .map(|e| e.decision())
            .unwrap_or(Decision::Commit)
    }

    pub fn foreign_inputs(&self) -> &HashSet<SubstateRequirement> {
        &self.foreign_inputs
    }

    pub fn local_inputs(&self) -> &IndexMap<SubstateRequirement, u32> {
        &self.local_inputs
    }

    pub fn outputs(&self) -> &HashSet<VersionedSubstateId> {
        &self.outputs
    }

    pub fn into_execution(self) -> Option<TransactionExecution> {
        self.execution
    }

    pub fn to_initial_evidence(&self, num_preshards: NumPreshards, num_committees: u32) -> Evidence {
        // if let Some(ref execution) = self.execution {
        //     return Evidence::from_inputs_and_outputs(execution.resolved_inputs(), execution.resulting_outputs());
        // }
        //
        // // CASE: One or more local inputs are not found, so the transaction is aborted.
        // if self.current_decision().is_abort() {
        //     return Evidence::from_inputs_and_outputs(
        //         self.execution
        //             .transaction()
        //             .all_inputs_iter()
        //             .map(|input| VersionedSubstateIdLockIntent::from_requirement(input, SubstateLockType::Read)),
        //         self.outputs
        //             .iter()
        //             .map(|id| VersionedSubstateIdLockIntent::output(id.clone())),
        //     );
        // }

        // TODO: We do not know if the inputs locks required are Read/Write. Either we allow the user to
        //       specify this or we can correct the locks after execution. Currently, this limitation
        //       prevents concurrent multi-shard read locks.
        let inputs = self
            .local_inputs()
            .iter()
            .map(|(requirement, version)| VersionedSubstateId::new(requirement.substate_id.clone(), *version))
            // TODO(correctness): to_zero_version is error prone when used in evidence and the correctness depends how it is used.
            // e.g. using it to determining which shard is involved is fine, but loading substate by the address is incorrect (v0 may or may not be the actual pledged substate)
            .chain(self.foreign_inputs().iter().map(|r| r.clone().or_zero_version()))
            .map(|id| VersionedSubstateIdLockIntent::write(id, true));

        let outputs = self
            .outputs()
            .iter()
            .cloned()
            .map(VersionedSubstateIdLockIntent::output);

        Evidence::from_inputs_and_outputs(num_preshards, num_committees, inputs, outputs)
    }
}
