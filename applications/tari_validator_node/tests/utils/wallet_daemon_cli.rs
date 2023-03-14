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

use serde::Serialize;
use tari_crypto::{
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    signatures::CommitmentSignature,
    tari_utilities::message_format::MessageFormat,
};
use tari_template_lib::prelude::ComponentAddress;
use tari_wallet_daemon_client::{
    types::{ClaimBurnRequest, ClaimBurnResponse},
    WalletDaemonClient,
};

use super::wallet_daemon::get_walletd_client;
use crate::TariWorld;

pub async fn claim_burn(
    world: &mut TariWorld,
    account_address: ComponentAddress,
    commitment: Vec<u8>,
    range_proof: Vec<u8>,
    ownership_proof: CommitmentSignature<RistrettoPublicKey, RistrettoSecretKey>,
    reciprocal_claim_public_key: RistrettoPublicKey,
    wallet_daemon_name: String,
) -> ClaimBurnResponse {
    #[derive(Serialize)]
    struct OwnershipProof {
        public_nonce: String,
        u: String,
        v: String,
    }

    #[derive(Serialize)]
    struct ClaimValue {
        commitment: String,
        ownership_proof: OwnershipProof,
        reciprocal_claim_public_key: String,
        range_proof: String,
    }

    let value = ClaimValue {
        commitment: commitment.to_base64().unwrap(),
        ownership_proof: OwnershipProof {
            public_nonce: ownership_proof.public_nonce().to_base64().unwrap(),
            u: ownership_proof.u().clone().to_base64().unwrap(),
            v: ownership_proof.v().to_base64().unwrap(),
        },
        reciprocal_claim_public_key: reciprocal_claim_public_key.to_base64().unwrap(),
        range_proof: range_proof.to_base64().unwrap(),
    };

    let claim_proof = serde_json::to_value(value).unwrap();

    let claim_burn_request = ClaimBurnRequest {
        account: account_address,
        claim_proof,
        fee: 1,
    };

    let mut client = get_wallet_daemon_client(world, wallet_daemon_name).await;
    client.claim_burn(claim_burn_request).await.unwrap()
}

async fn get_wallet_daemon_client(world: &TariWorld, wallet_daemon_name: String) -> WalletDaemonClient {
    let port = world.wallet_daemons.get(&wallet_daemon_name).unwrap().json_rpc_port;
    get_walletd_client(port).await
}
