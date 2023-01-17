//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use tari_bor::{borsh, Decode, Encode};

pub type ConfidentialBucketId = u32;

#[derive(Debug, Clone, Decode, Encode)]
pub struct ConfidentialBucket {
    id: ConfidentialBucketId,
}
