//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::io;

use crate::{
    codecs::{DbDecoder, DbEncoder},
    error::RocksDbStorageError,
};

#[derive(Debug, Clone, Copy, strum_macros::FromRepr)]
#[repr(u8)]
pub enum KeyPrefix {
    Blocks = 0,
    BlockEpochHeightIndex = 1,
    BlockDiffs = 2,
    BlockDiffsBySubstateId = 3,
    BlockTransactionExecutions = 4,
    BlockTransactionExecutionByBlockIdIndex = 5,
    ProposalCertificates = 6,
    TimeoutCertificates = 7,
    PendingChainIndex = 8,
    PendingParentChildIndex = 9,
    CommittedParentChildIndex = 10,
    DiagnosticsNoVotes = 11,
    EpochCheckpoints = 12,
    // 13 is unused
    FinalizedTransactionLinks = 14,
    ForeignParkedBlocks = 15,
    MissingTransactions = 16,
    ForeignProposals = 17,
    ForeignProposalsEpochIndex = 18,
    ForeignProposalsProposedInBlockIndex = 19,
    ForeignProposalsUnconfirmedIndex = 20,
    ForeignSubstatePledges = 21,
    LockConflicts = 22,
    LockConflictBlockIdIndex = 23,
    Transactions = 24,
    TransactionPool = 25,
    TransactionPoolStateUpdates = 26,
    TransactionPoolStateUpdateDebugHistory = 27,
    ForeignProposalMissingTransactions = 28,
    MissingTransactionBlockIdIndex = 29,
    ParkedBlocks = 30,
    PendingStateTreeDiff = 31,
    SubstateLocks = 32,
    SubstateLockHeadIndex = 33,
    SubstateLocksBlockIdIndex = 34,
    SubstateLockSubstateIdIndex = 35,
    Substates = 36,
    SubstatesHeadIndex = 37,
    SubstatesUnprunedDownedValuesIndex = 38,
    StateTransitions = 39,
    StateTree = 40,
    StateTreeStaleTreeNodesIndex = 41,
    StateTreeShardVersion = 42,
    ValidatorNodeEpochStats = 43,
    RollbackHistory = 44,
    FinalizedTransactionEpochIndex = 45,
}

impl KeyPrefix {
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

pub trait Prefixed {
    fn prefix() -> Option<u8>;
}

impl Prefixed for () {
    fn prefix() -> Option<u8> {
        None
    }
}

pub struct PrefixCodec<P, C> {
    _phantom: std::marker::PhantomData<P>,
    inner: C,
}

impl<C: DbEncoder<T>, P: Prefixed, T> DbEncoder<T> for PrefixCodec<P, C> {
    fn encode_len(&self, value: &T) -> Result<usize, RocksDbStorageError> {
        self.inner
            .encode_len(value)
            .map(|len| len + usize::from(P::prefix().is_some()))
    }

    fn encode_into<W: io::Write>(&self, value: &T, writer: &mut W) -> Result<(), RocksDbStorageError> {
        if let Some(p) = P::prefix() {
            writer.write_all(&[p]).map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow::anyhow!("PrefixCodec: Failed to write prefix: {}", e),
            })?;
        }
        self.inner.encode_into(value, writer)
    }
}

impl<C: DbDecoder<T>, P: Prefixed, T> DbDecoder<T> for PrefixCodec<P, C> {
    fn decode(&self, bytes: &[u8]) -> Result<(T, usize), RocksDbStorageError> {
        let mut consumed = 0;
        if let Some(p) = P::prefix() {
            let prefix = *bytes.first().ok_or_else(|| RocksDbStorageError::MalformedData {
                operation: "PrefixCodec: read prefix",
                details: "PrefixCodec: Not enough bytes for prefix".to_string(),
            })?;
            if prefix != p {
                return Err(RocksDbStorageError::MalformedData {
                    operation: "PrefixCodec: validate prefix",
                    details: format!("PrefixCodec: Invalid prefix byte. Expected {}, got {}", p, prefix),
                });
            }
            consumed = 1;
        }
        let (value, inner_consumed) = self.inner.decode(&bytes[consumed..])?;
        Ok((value, consumed + inner_consumed))
    }
}

impl<P, C> Default for PrefixCodec<P, C>
where C: Default
{
    fn default() -> Self {
        Self {
            _phantom: std::marker::PhantomData,
            inner: C::default(),
        }
    }
}

#[macro_export]
macro_rules! prefixed {
    ($name:ident, $prefix:expr) => {
        pub struct $name;
        impl $crate::codecs::Prefixed for $name {
            fn prefix() -> Option<u8> {
                Some($prefix.as_u8())
            }
        }
    };
}
