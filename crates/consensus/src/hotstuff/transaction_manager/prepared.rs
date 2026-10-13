//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus_types::Decision;
use tari_ootle_storage::consensus_models::{Evidence, TransactionExecution};

use crate::hotstuff::substate_store::LockStatus;

#[derive(Debug)]
pub enum PreparedTransaction {
    LocalOnly(Box<LocalPreparedTransaction>),
    MultiShard(MultiShardPreparedTransaction),
    /// The transaction's local inputs do not fit in what the block being proposed can still pledge to foreign
    /// shard groups. Nothing was locked for it.
    PledgeBudgetReached {
        total_bytes: usize,
        max_bytes: usize,
    },
}

impl PreparedTransaction {
    pub fn new_local_accept(executed: TransactionExecution, lock_status: LockStatus) -> Self {
        Self::LocalOnly(Box::new(LocalPreparedTransaction::Accept {
            execution: executed,
            lock_status,
        }))
    }

    pub fn new_local_early_abort(execution: TransactionExecution) -> Self {
        Self::LocalOnly(Box::new(LocalPreparedTransaction::EarlyAbort { execution }))
    }

    pub fn lock_status(&self) -> &LockStatus {
        static DEFAULT_LOCK_STATUS: LockStatus = LockStatus::new();
        match self {
            Self::LocalOnly(local) => match &**local {
                LocalPreparedTransaction::Accept { lock_status, .. } => lock_status,
                LocalPreparedTransaction::EarlyAbort { .. } => &DEFAULT_LOCK_STATUS,
            },
            Self::MultiShard(multishard) => &multishard.lock_status,
            Self::PledgeBudgetReached { .. } => &DEFAULT_LOCK_STATUS,
        }
    }

    pub fn into_lock_status(self) -> LockStatus {
        match self {
            Self::LocalOnly(local) => match *local {
                LocalPreparedTransaction::Accept { lock_status, .. } => lock_status,
                LocalPreparedTransaction::EarlyAbort { .. } => LockStatus::new(),
            },
            Self::MultiShard(multishard) => multishard.lock_status,
            Self::PledgeBudgetReached { .. } => LockStatus::new(),
        }
    }

    pub fn new_multishard_executed(execution: TransactionExecution, lock_status: LockStatus) -> Self {
        Self::MultiShard(MultiShardPreparedTransaction {
            execution_or_evidence: EvidenceOrExecution::Execution {
                execution: Box::new(execution),
            },
            lock_status,
        })
    }

    pub fn new_multishard_evidence(initial_evidence: Evidence, lock_status: LockStatus) -> Self {
        Self::MultiShard(MultiShardPreparedTransaction {
            execution_or_evidence: EvidenceOrExecution::Evidence {
                evidence: initial_evidence,
            },
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
pub enum EvidenceOrExecution {
    Evidence { evidence: Evidence },
    Execution { execution: Box<TransactionExecution> },
}

impl EvidenceOrExecution {
    pub fn execution(&self) -> Option<&TransactionExecution> {
        match self {
            Self::Evidence { .. } => None,
            Self::Execution { execution, .. } => Some(execution),
        }
    }
}

#[derive(Debug)]
pub struct MultiShardPreparedTransaction {
    execution_or_evidence: EvidenceOrExecution,
    lock_status: LockStatus,
}

impl MultiShardPreparedTransaction {
    pub fn current_decision(&self) -> Decision {
        self.execution_or_evidence
            .execution()
            .map(|e| e.decision())
            .unwrap_or(Decision::Commit)
    }

    pub fn into_evidence_or_execution(self) -> EvidenceOrExecution {
        self.execution_or_evidence
    }
}
