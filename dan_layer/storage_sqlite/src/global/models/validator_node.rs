//  Copyright 2022. The Tari Project
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

use tari_dan_storage::global::DbValidatorNode;

use crate::global::schema::*;

#[derive(Queryable, Identifiable)]
pub struct ValidatorNode {
    pub id: i32,
    pub public_key: Vec<u8>,
    pub shard_key: Vec<u8>,
    pub epoch: i64,
}

impl From<ValidatorNode> for DbValidatorNode {
    fn from(vn: ValidatorNode) -> Self {
        Self {
            shard_key: vn.shard_key,
            public_key: vn.public_key,
            epoch: vn.epoch as u64,
        }
    }
}

#[derive(Insertable)]
#[table_name = "validator_nodes"]
pub struct NewValidatorNode {
    pub public_key: Vec<u8>,
    pub shard_key: Vec<u8>,
    pub epoch: i64,
}

impl From<DbValidatorNode> for NewValidatorNode {
    fn from(vn: DbValidatorNode) -> Self {
        Self {
            shard_key: vn.shard_key,
            public_key: vn.public_key,
            epoch: vn.epoch as i64,
        }
    }
}
