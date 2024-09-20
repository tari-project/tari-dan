//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use indexmap::IndexMap;
use tari_dan_common_types::{committee::CommitteeInfo, SubstateRequirement, VersionedSubstateId};
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

    pub fn known_outputs(&self) -> &HashSet<VersionedSubstateId> {
        &self.outputs
    }

    pub fn into_execution(self) -> Option<TransactionExecution> {
        self.execution
    }

    pub fn to_initial_evidence(&self, local_committee_info: &CommitteeInfo) -> Evidence {
        // TODO: We do not know if the inputs locks required are Read/Write. Either we allow the user to
        //       specify this or we can correct the locks after execution. Currently, this limitation
        //       prevents concurrent multi-shard read locks.
        let inputs = self
            .local_inputs()
            .iter()
            .map(|(requirement, version)| VersionedSubstateId::new(requirement.substate_id.clone(), *version))
            .map(|id| VersionedSubstateIdLockIntent::write(id, true));

        let outputs = self
            .known_outputs()
            .iter()
            .cloned()
            .map(VersionedSubstateIdLockIntent::output);

        let mut evidence = Evidence::from_inputs_and_outputs(
            local_committee_info.num_preshards(),
            local_committee_info.num_committees(),
            inputs,
            outputs,
        );

        // Add foreign involved shard groups without adding any substates (because we do not know the pledged version
        // yet)
        self.foreign_inputs()
            .iter()
            .map(|r| {
                r.to_substate_address_zero_version().to_shard_group(
                    local_committee_info.num_preshards(),
                    local_committee_info.num_committees(),
                )
            })
            .for_each(|sg| {
                evidence.add_shard_group(sg);
            });

        evidence
    }
}
