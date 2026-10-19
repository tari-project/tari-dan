//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_ootle_common_types::hashing;
use tari_sidechain::FixedHash;

crate::create_hash_type!(
    /// The ID of a block in the Tari consensus protocol.
    #[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, type = "string"))]
    BlockId
);

impl BlockId {
    pub fn from_parent_and_header_hash(parent_id: &BlockId, header_hash: &FixedHash) -> Self {
        // The zero block is a special case. It has no parent and its ID is always zero.
        if *header_hash == FixedHash::zero() && parent_id.is_zero() {
            return BlockId::zero();
        }

        hashing::block_hasher()
            .chain(parent_id)
            .chain(header_hash)
            .finalize_into_array()
            .into()
    }
}
