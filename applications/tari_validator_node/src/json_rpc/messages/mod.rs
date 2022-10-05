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
//

use serde::Deserialize;
use tari_common_types::types::PublicKey;
use tari_crypto::ristretto::RistrettoSchnorr;
use tari_dan_common_types::{serde_with, ShardId};
use tari_dan_core::models::Epoch;
use tari_template_lib::args::Arg;

#[derive(Deserialize, Debug, Clone)]
pub struct SubmitTransactionRequest {
    pub instructions: Vec<InstructionRequest>,
    pub signature: RistrettoSchnorr,
    pub sender_public_key: PublicKey,
    pub num_new_components: u8,
}

#[derive(Deserialize, Debug, Clone)]
pub struct InstructionRequest {
    #[serde(deserialize_with = "serde_with::hex::deserialize")]
    pub package_address: [u8; 32],
    pub template: String,
    pub function: String,
    pub args: Vec<Arg>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct GetCommitteeRequest {
    pub epoch: Epoch,
    pub shard_id: ShardId,
}
