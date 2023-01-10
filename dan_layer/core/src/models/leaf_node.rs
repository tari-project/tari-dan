//  Copyright 2022, The Tari Project
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

use crate::models::{NodeHeight, TreeNodeHash};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafNode {
    hash: TreeNodeHash,
    height: NodeHeight,
    payload_height: NodeHeight,
}

impl LeafNode {
    pub(crate) fn is_genesis(&self) -> bool {
        self.hash.is_zero()
    }
}

impl LeafNode {
    pub fn genesis() -> Self {
        Self {
            hash: TreeNodeHash::zero(),
            height: NodeHeight(0),
            payload_height: NodeHeight(0),
        }
    }

    pub fn new(hash: TreeNodeHash, height: NodeHeight, payload_height: NodeHeight) -> Self {
        Self {
            hash,
            height,
            payload_height,
        }
    }

    pub fn hash(&self) -> &TreeNodeHash {
        &self.hash
    }

    pub fn height(&self) -> NodeHeight {
        self.height
    }

    pub fn payload_height(&self) -> NodeHeight {
        self.payload_height
    }
}
