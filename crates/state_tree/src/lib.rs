//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub use tari_jellyfish::*;

pub mod cbor;

mod error;
pub use error::*;

pub mod key_mapper;
pub mod memory_store;

mod jmt_hash_scheme;
pub use jmt_hash_scheme::*;

mod shard_state_leaf;
pub use shard_state_leaf::*;

mod indexed_diff;
pub use indexed_diff::*;

mod staged_store;
pub use staged_store::*;

mod substate_proof;
pub use substate_proof::*;

mod traits;
pub use traits::*;

mod tree;

pub use tree::*;

/// The payload type used in the state tree. This is a reference to a particular substate (i.e. SubstateAddress).
pub type StateTreePayload = tari_ootle_common_types::SubstateAddress;
