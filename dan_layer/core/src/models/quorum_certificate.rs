// Copyright 2021. The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use digest::Digest;
use tari_crypto::hash::blake2::Blake256;

use crate::{
    models::{HotStuffMessageType, TreeNodeHash, ValidatorSignature, ViewId},
    storage::chain::DbQc,
};

#[derive(Debug, Clone)]
pub struct QuorumCertificate {
    message_type: HotStuffMessageType,
    // Cache the node hash
    node_hash: TreeNodeHash,
    // cache the node height
    node_height: u32,
    shard: u32,
    epoch: u32,
    involved_shards: Vec<u32>,
    signatures: Vec<ValidatorSignature>,
}

impl QuorumCertificate {
    pub fn new(
        message_type: HotStuffMessageType,
        node_height: u32,
        node_hash: TreeNodeHash,
        shard: u32,
        epoch: u32,
        involved_shards: Vec<u32>,
        signatures: Vec<ValidatorSignature>,
    ) -> Self {
        Self {
            message_type,
            node_hash,
            shard,
            epoch,
            involved_shards,
            node_height,
            signatures,
        }
    }

    pub fn genesis() -> Self {
        Self {
            message_type: HotStuffMessageType::Genesis,
            node_hash: TreeNodeHash::zero(),
            shard: 0,
            epoch: 0,
            node_height: 0,
            involved_shards: vec![],
            signatures: vec![],
        }
    }

    pub fn shard(&self) -> u32 {
        self.shard
    }

    pub fn involved_shards(&self) -> &[u32] {
        self.involved_shards.as_slice()
    }

    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    pub fn node_hash(&self) -> &TreeNodeHash {
        &self.node_hash
    }

    pub fn node_height(&self) -> u32 {
        self.node_height
    }

    pub fn message_type(&self) -> HotStuffMessageType {
        self.message_type
    }

    pub fn signature(&self) -> &[ValidatorSignature] {
        self.signatures.as_slice()
    }

    // pub fn combine_sig(&mut self, partial_sig: &ValidatorSignature) {
    //     self.signatures = match &self.signatures {
    //         None => Some(partial_sig.clone()),
    //         Some(s) => Some(s.combine(partial_sig)),
    //     };
    // }

    pub fn matches(&self, message_type: HotStuffMessageType, view_id: ViewId) -> bool {
        todo!("Update as this has changed from view number to height")
        // from hotstuf spec
        // self.message_type() == message_type && view_id == self.view_number()
    }

    pub fn as_bytes(&self) -> Vec<u8> {
        let mut result = Blake256::new()
            .chain([self.message_type.as_u8()])
            .chain(self.node_hash.as_bytes())
            .chain(self.node_height.to_le_bytes())
            .chain(self.shard.to_le_bytes())
            .chain((self.signatures.len() as u64).to_le_bytes());

        for sig in &self.signatures {
            result = result.chain(sig.to_bytes());
        }
        result = result.chain((self.involved_shards.len() as u32).to_le_bytes());
        for shard in &self.involved_shards {
            result = result.chain((*shard).to_le_bytes());
        }
        result.finalize().to_vec()
    }
}

impl From<DbQc> for QuorumCertificate {
    fn from(rec: DbQc) -> Self {
        // Self {
        //     message_type: rec.message_type,
        //     node_hash: rec.node_hash,
        //     view_number: rec.view_number,
        //     signatures: rec.signature,
        // }
        todo!()
    }
}
