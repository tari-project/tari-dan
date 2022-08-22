//  Copyright 2021. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{convert::TryFrom, fmt::Debug};

use tari_common_types::types::FixedHash;

use crate::models::ConsensusHash;

// TODO: Rename to Command - most of the hotstuff docs refers to this as command
pub trait Payload: Debug + Clone + Send + Sync + ConsensusHash {
    fn involved_shards(&self) -> &[u32];
    fn to_id(&self) -> PayloadId {
        PayloadId::new(FixedHash::try_from(self.consensus_hash().to_vec()).unwrap())
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct PayloadId {
    id: FixedHash,
}

impl PayloadId {
    pub fn new(id: FixedHash) -> Self {
        Self { id }
    }

    pub fn zero() -> Self {
        Self { id: FixedHash::zero() }
    }

    pub fn as_slice(&self) -> &[u8] {
        self.id.as_slice()
    }
}

// impl Payload for &str {
//     fn involved_shards(&self) -> Vec<u32> {
//         self.as_bytes()
//     }
// }

// impl Payload for String {
//     fn involved_shards(&self) -> Vec<u32> {
//         vec![0]
//     }
// }

impl ConsensusHash for (String, Vec<u32>) {
    fn consensus_hash(&self) -> &[u8] {
        self.0.consensus_hash()
    }
}

impl Payload for (String, Vec<u32>) {
    fn involved_shards(&self) -> &[u32] {
        &self.1
    }
}
