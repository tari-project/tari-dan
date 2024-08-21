//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use indexmap::IndexMap;
use tari_dan_storage::consensus_models::{
    Decision,
    Evidence,
    ExecutedTransaction,
    SubstateLockType,
    TransactionExecution,
    TransactionRecord,
    VersionedSubstateIdLockIntent,
};
use tari_engine_types::{
    commit_result::RejectReason,
    substate::{Substate, SubstateId},
};
use tari_transaction::{SubstateRequirement, VersionedSubstateId};

#[derive(Debug, Clone)]
pub enum PreparedTransaction {
    LocalOnly(LocalPreparedTransaction),
    MultiShard(MultiShardPreparedTransaction),
}

impl PreparedTransaction {
    pub fn new_local_accept(transaction: ExecutedTransaction) -> Self {
        Self::LocalOnly(LocalPreparedTransaction::Accept(transaction))
    }

    pub fn new_local_early_abort(transaction: TransactionRecord) -> Self {
        Self::LocalOnly(LocalPreparedTransaction::EarlyAbort { transaction })
    }

    pub fn new_multishard(
        transaction: TransactionRecord,
        local_inputs: IndexMap<SubstateId, Substate>,
        foreign_inputs: HashSet<SubstateRequirement>,
        outputs: HashSet<VersionedSubstateId>,
    ) -> Self {
        Self::MultiShard(MultiShardPreparedTransaction {
            transaction,
            local_inputs,
            foreign_inputs,
            outputs,
        })
    }

    pub fn set_abort_reason(&mut self, reason: RejectReason) {
        match self {
            Self::LocalOnly(local) => {
                local.set_abort_reason(reason);
            },
            Self::MultiShard(multishard) => {
                multishard.set_abort_reason(reason);
            },
        }
    }
}

#[derive(Debug, Clone)]
pub enum LocalPreparedTransaction {
    Accept(ExecutedTransaction),
    EarlyAbort { transaction: TransactionRecord },
}

impl LocalPreparedTransaction {
    pub fn set_abort_reason(&mut self, reason: RejectReason) {
        match self {
            Self::Accept(accept) => {
                accept.set_abort_reason(reason);
            },
            Self::EarlyAbort { transaction } => {
                transaction.set_abort_reason(reason);
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct MultiShardPreparedTransaction {
    transaction: TransactionRecord,
    local_inputs: IndexMap<SubstateId, Substate>,
    outputs: HashSet<VersionedSubstateId>,
    foreign_inputs: HashSet<SubstateRequirement>,
}

impl MultiShardPreparedTransaction {
    pub fn transaction(&self) -> &TransactionRecord {
        &self.transaction
    }

    pub fn current_decision(&self) -> Decision {
        self.transaction.current_decision()
    }

    pub fn foreign_inputs(&self) -> &HashSet<SubstateRequirement> {
        &self.foreign_inputs
    }

    pub fn local_inputs(&self) -> &IndexMap<SubstateId, Substate> {
        &self.local_inputs
    }

    pub fn outputs(&self) -> &HashSet<VersionedSubstateId> {
        &self.outputs
    }

    pub fn set_abort_reason(&mut self, reason: RejectReason) {
        self.transaction.set_abort_reason(reason);
    }

    pub fn into_execution(self) -> Option<TransactionExecution> {
        self.transaction.into_execution()
    }

    pub fn to_initial_evidence(&self) -> Evidence {
        if let Some(resolved_inputs) = self.transaction.resolved_inputs() {
            return Evidence::from_inputs_and_outputs(
                resolved_inputs,
                self.transaction
                    .resulting_outputs()
                    .expect("invariant: resulting_outputs is Some if resolved_inputs is Some"),
            );
        }

        // CASE: One or more local inputs are not found, so the transaction is aborted. We have no resolved inputs.
        if self.transaction.current_decision().is_abort() {
            return Evidence::from_inputs_and_outputs(
                self.transaction
                    .transaction()
                    .all_inputs_iter()
                    .map(|input| input.or_zero_version())
                    .map(|id| VersionedSubstateIdLockIntent::new(id, SubstateLockType::Read)),
                self.outputs
                    .iter()
                    .map(|id| VersionedSubstateIdLockIntent::new(id.clone(), SubstateLockType::Output)),
            );
        }

        // TODO: We do not know if the inputs locks required are Read/Write. Either we allow the user to
        //       specify this or we can correct the locks after execution. Currently, this limitation
        //       prevents concurrent multi-shard read locks.
        let inputs = self
            .local_inputs()
            .iter()
            .map(|(substate_id, substate)| VersionedSubstateId::new(substate_id.clone(), substate.version()))
            // TODO(correctness): to_zero_version is error prone when used in evidence and the correctness depends how it is used.
            // e.g. using it to determining which shard is involved is fine, but loading substate by the address is incorrect (v0 may or may not be the actual pledged substate)
            .chain(self.foreign_inputs().iter().map(|r| r.clone().or_zero_version()))
            .map(|id| VersionedSubstateIdLockIntent::new(id, SubstateLockType::Write));

        let outputs = self
            .outputs()
            .iter()
            .cloned()
            .map(|id| VersionedSubstateIdLockIntent::new(id, SubstateLockType::Output));

        Evidence::from_inputs_and_outputs(inputs, outputs)
    }
}
