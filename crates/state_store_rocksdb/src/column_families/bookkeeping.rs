//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use minicbor::{CborLen, Decode, Encode};
use serde::{Deserialize, Serialize};
use tari_consensus_types::{
    BlockId,
    HighPc,
    HighTc,
    HighestSeenBlock,
    LastExecuted,
    LastProposed,
    LastSentNewView,
    LastSentVote,
    LastVoted,
    LeafBlock,
    LockedBlock,
};
use tari_ootle_common_types::{Epoch, NodeHeight};

use crate::{
    codecs::{ByteColumn, ColumnCodec, DefaultCodec, NumberCodec},
    column_families::cf_names,
    traits::Cf,
};

enum BookKeepingKey {
    /// The migration version of the database
    DatabaseMigrationVersion,
    /// The last voted block
    LastVoted,
    /// The last executed block
    LastExecuted,
    /// The last proposed block
    LastProposed,
    /// The last sent vote
    LastSentVote,
    /// The last committed block
    CommitBlock,
    /// The last leaf block
    LeafBlock,
    /// The last locked block
    LockedBlock,
    /// The last high proposal certificate
    HighQc,
    /// The last high timeout certificate
    HighTc,
    /// The highest block seen by the node
    HighestSeenBlock,
    /// The last sent new view message
    LastSentNewView,
    /// The last committed end-of-epoch block
    LastCommittedEpochEnd,
}

impl BookKeepingKey {
    const fn as_byte(&self) -> u8 {
        match self {
            Self::DatabaseMigrationVersion => 0,
            Self::LastVoted => 1,
            Self::LastExecuted => 2,
            Self::LastProposed => 3,
            Self::LastSentVote => 4,
            Self::CommitBlock => 5,
            Self::LockedBlock => 6,
            Self::LeafBlock => 7,
            Self::HighQc => 8,
            Self::HighTc => 9,
            Self::HighestSeenBlock => 10,
            Self::LastSentNewView => 11,
            Self::LastCommittedEpochEnd => 12,
        }
    }
}

/// The database schema version that this build expects a database to be at.
///
/// The migration steps that bring a database up to this version live in the validator node, but the constant lives
/// here because it describes the on-disk schema rather than any one binary. Tools that write to a database directly
/// must check the stored [`DatabaseMigrationVersion`] against it first: writing through the current key encodings to a
/// database still at an older version silently misses the rows that a pending migration has yet to rewrite.
///
/// A node refuses a stored version above this one. It migrates one below it where a migration from that version
/// exists, and refuses it otherwise.
/// The numbering restarted with the v0.42.0 network reset at the same values the schema used before it (0 to 2), so a
/// database written before v0.42.0 cannot be told apart from an older post-reset one by its stored version. Version 4
/// marks the next network reset after v0.42.0, and every version below it is refused. Any future reset or other
/// incompatible on-disk change must bump this, so that the node refuses databases it cannot use rather than reading
/// them under the wrong schema.
pub const CURRENT_SCHEMA_VERSION: u64 = 4;

pub struct DatabaseMigrationVersion;

impl Cf for DatabaseMigrationVersion {
    type Key = ByteColumn<{ BookKeepingKey::DatabaseMigrationVersion.as_byte() }>;
    type KeyCodec = ColumnCodec;
    type Prefix = ();
    type Value = u64;
    type ValueCodec = NumberCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::BOOKKEEPING
    }
}

pub struct LastVotedCf;

impl Cf for LastVotedCf {
    type Key = ByteColumn<{ BookKeepingKey::LastVoted.as_byte() }>;
    type KeyCodec = ColumnCodec;
    type Prefix = ();
    type Value = LastVoted;
    type ValueCodec = DefaultCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::BOOKKEEPING
    }
}

pub struct LastExecutedCf;

impl Cf for LastExecutedCf {
    type Key = ByteColumn<{ BookKeepingKey::LastExecuted.as_byte() }>;
    type KeyCodec = ColumnCodec;
    type Prefix = ();
    type Value = LastExecuted;
    type ValueCodec = DefaultCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::BOOKKEEPING
    }
}

pub struct LastProposedCf;

impl Cf for LastProposedCf {
    type Key = ByteColumn<{ BookKeepingKey::LastProposed.as_byte() }>;
    type KeyCodec = ColumnCodec;
    type Prefix = ();
    type Value = LastProposed;
    type ValueCodec = DefaultCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::BOOKKEEPING
    }
}

pub struct LastSentVoteCf;

impl Cf for LastSentVoteCf {
    type Key = ByteColumn<{ BookKeepingKey::LastSentVote.as_byte() }>;
    type KeyCodec = ColumnCodec;
    type Prefix = ();
    type Value = LastSentVote;
    type ValueCodec = DefaultCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::BOOKKEEPING
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, CborLen)]
pub struct CommitBlock {
    #[n(0)]
    pub height: NodeHeight,
    #[n(1)]
    pub block_id: BlockId,
    #[n(2)]
    pub parent_id: BlockId,
    #[n(3)]
    pub epoch: Epoch,
}

pub struct CommitBlockCf;

impl Cf for CommitBlockCf {
    type Key = ByteColumn<{ BookKeepingKey::CommitBlock.as_byte() }>;
    type KeyCodec = ColumnCodec;
    type Prefix = ();
    type Value = CommitBlock;
    type ValueCodec = DefaultCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::BOOKKEEPING
    }
}

pub struct LeafBlockCf;

impl Cf for LeafBlockCf {
    type Key = ByteColumn<{ BookKeepingKey::LeafBlock.as_byte() }>;
    type KeyCodec = ColumnCodec;
    type Prefix = ();
    type Value = LeafBlock;
    type ValueCodec = DefaultCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::BOOKKEEPING
    }
}

pub struct LastCommittedEpochEndCf;

impl Cf for LastCommittedEpochEndCf {
    type Key = ByteColumn<{ BookKeepingKey::LastCommittedEpochEnd.as_byte() }>;
    type KeyCodec = ColumnCodec;
    type Prefix = ();
    type Value = LeafBlock;
    type ValueCodec = DefaultCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::BOOKKEEPING
    }
}

pub struct LockedBlockCf;

impl Cf for LockedBlockCf {
    type Key = ByteColumn<{ BookKeepingKey::LockedBlock.as_byte() }>;
    type KeyCodec = ColumnCodec;
    type Prefix = ();
    type Value = LockedBlock;
    type ValueCodec = DefaultCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::BOOKKEEPING
    }
}

pub struct HighPcCf;

impl Cf for HighPcCf {
    type Key = ByteColumn<{ BookKeepingKey::HighQc.as_byte() }>;
    type KeyCodec = ColumnCodec;
    type Prefix = ();
    type Value = HighPc;
    type ValueCodec = DefaultCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::BOOKKEEPING
    }
}

pub struct HighTcCf;

impl Cf for HighTcCf {
    type Key = ByteColumn<{ BookKeepingKey::HighTc.as_byte() }>;
    type KeyCodec = ColumnCodec;
    type Prefix = ();
    type Value = HighTc;
    type ValueCodec = DefaultCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::BOOKKEEPING
    }
}

pub struct HighestSeenBlockCf;

impl Cf for HighestSeenBlockCf {
    type Key = ByteColumn<{ BookKeepingKey::HighestSeenBlock.as_byte() }>;
    type KeyCodec = ColumnCodec;
    type Prefix = ();
    type Value = HighestSeenBlock;
    type ValueCodec = DefaultCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::BOOKKEEPING
    }
}

pub struct LastSentNewViewCf;

impl Cf for LastSentNewViewCf {
    type Key = ByteColumn<{ BookKeepingKey::LastSentNewView.as_byte() }>;
    type KeyCodec = ColumnCodec;
    type Prefix = ();
    type Value = LastSentNewView;
    type ValueCodec = DefaultCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::BOOKKEEPING
    }
}
