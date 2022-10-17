//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tari_common_types::types::{FixedHash, PublicKey};
use tari_dan_common_types::{serde_with, Epoch, ShardId, SubstateState};
use tari_engine_types::{instruction::Instruction, signature::InstructionSignature};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateRegistrationRequest {
    pub template_name: String,
    pub template_version: u16,
    pub repo_url: String,
    #[serde(with = "serde_with::base64")]
    pub commit_hash: Vec<u8>,
    #[serde(with = "serde_with::base64")]
    pub binary_sha: Vec<u8>,
    pub binary_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateRegistrationResponse {
    #[serde(with = "serde_with::base64")]
    pub template_address: Vec<u8>,
    pub transaction_id: u64,
}

/// A request to submit a transaction
/// ```json
/// instructions": [{
///    "type": "CallFunction",
///    "template_address": "55886cfee6e91503b7f1df2dc6d11951b53db64733521595c3505747b83be277",
///    "function": "new",
///    "args": [{
///       "type":"Literal",
///       "value": "1232"
///    }]
///  }],
///  "signature": {
///    "public_nonce": "90392b9cebd7bf7d693f938911ccd3fb735a6cf24fcf1341a2edca38c560b563",
///    "signature": "90392b9cebd7bf7d693f938911ccd3fb735a6cf24fcf1341a2edca38c560b563"
///   },
///   "fee": 1,
///   "sender_public_key": "90392b9cebd7bf7d693f938911ccd3fb735a6cf24fcf1341a2edca38c560b563",
///   "num_new_components": 1
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitTransactionRequest {
    pub instructions: Vec<Instruction>,
    pub signature: InstructionSignature,
    pub fee: u64,
    pub sender_public_key: PublicKey,
    pub num_new_components: u8,
    /// Set to true to wait for the transaction to complete before returning
    #[serde(default)]
    pub wait_for_result: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitTransactionResponse {
    #[serde(with = "serde_with::hex")]
    pub hash: FixedHash,
    pub changes: HashMap<ShardId, SubstateState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetCommitteeRequest {
    pub epoch: Epoch,
    pub shard_id: ShardId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetShardKey {
    pub height: u64,
    pub public_key: PublicKey,
}
