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

use tari_common_types::types::PublicKey;
use tari_dan_common_types::{shard::Shard, Epoch, NodeAddressable, SubstateAddress};
use tari_dan_storage::global::models::ValidatorNode;
use tari_utilities::ByteArray;

use crate::{
    error::SqliteStorageError,
    global::{schema::*, serialization::deserialize_json},
};

#[derive(Queryable, Identifiable)]
#[diesel(table_name = validator_nodes)]
pub struct DbValidatorNode {
    pub id: i32,
    pub public_key: Vec<u8>,
    pub shard_key: Vec<u8>,
    pub epoch: i64,
    pub committee_bucket: Option<i64>,
    pub fee_claim_public_key: Vec<u8>,
    pub address: String,
}
impl<TAddr: NodeAddressable> TryFrom<DbValidatorNode> for ValidatorNode<TAddr> {
    type Error = SqliteStorageError;

    fn try_from(vn: DbValidatorNode) -> Result<Self, Self::Error> {
        Ok(Self {
            shard_key: SubstateAddress::try_from(vn.shard_key).map_err(|_| {
                SqliteStorageError::MalformedDbData(format!("Invalid shard id in validator node record id={}", vn.id))
            })?,
            address: deserialize_json(&vn.address)?,
            public_key: PublicKey::from_canonical_bytes(&vn.public_key).map_err(|_| {
                SqliteStorageError::MalformedDbData(format!("Invalid public key in validator node record id={}", vn.id))
            })?,
            epoch: Epoch(vn.epoch as u64),
            committee_shard: vn.committee_bucket.map(|v| v as u32).map(Shard::from),

            fee_claim_public_key: PublicKey::from_canonical_bytes(&vn.fee_claim_public_key).map_err(|_| {
                SqliteStorageError::MalformedDbData(format!(
                    "Invalid fee claim public key in validator node record id={}",
                    vn.id
                ))
            })?,
        })
    }
}

#[derive(Insertable)]
#[diesel(table_name = validator_nodes)]
pub struct NewValidatorNode {
    pub public_key: Vec<u8>,
    pub shard_key: Vec<u8>,
    pub epoch: i64,
    pub fee_claim_public_key: Vec<u8>,
}
