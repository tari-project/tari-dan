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

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde_json::json;
use tari_crypto::{
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    signatures::CommitmentSignature,
    tari_utilities::ByteArray,
};
use tari_dan_wallet_sdk::apis::jwt::{JrpcPermission, JrpcPermissions};
use tari_template_lib::{models::Amount, prelude::ResourceAddress};
use tari_wallet_daemon_client::{
    types::{
        AccountsCreateRequest,
        AuthLoginAcceptRequest,
        AuthLoginRequest,
        ClaimBurnRequest,
        ClaimBurnResponse,
        TransferRequest,
        TransferResponse,
    },
    ComponentAddressOrName,
    WalletDaemonClient,
};

use super::{helpers::add_substate_addresses, validator_node_cli::get_key_manager, wallet_daemon::get_walletd_client};
use crate::TariWorld;

pub async fn claim_burn(
    world: &mut TariWorld,
    account_name: String,
    commitment: Vec<u8>,
    range_proof: Vec<u8>,
    ownership_proof: CommitmentSignature<RistrettoPublicKey, RistrettoSecretKey>,
    reciprocal_claim_public_key: RistrettoPublicKey,
    wallet_daemon_name: String,
) -> ClaimBurnResponse {
    let mut client = get_wallet_daemon_client(world, wallet_daemon_name).await;

    let claim_burn_request = ClaimBurnRequest {
        account: Some(ComponentAddressOrName::Name(account_name)),
        claim_proof: json!({
            "commitment": BASE64.encode(commitment.as_bytes()),
            "ownership_proof": {
                "public_nonce": BASE64.encode(ownership_proof.public_nonce().as_bytes()),
                "u": BASE64.encode(ownership_proof.u().as_bytes()),
                "v": BASE64.encode(ownership_proof.v().as_bytes())
            },
            "reciprocal_claim_public_key": BASE64.encode(reciprocal_claim_public_key.as_bytes()),
            "range_proof": BASE64.encode(range_proof.as_bytes()),
        }),
        fee: Some(Amount(1)),
    };

    let auth_response = client
        .auth_request(AuthLoginRequest {
            permissions: JrpcPermissions(vec![JrpcPermission::Admin]),
        })
        .await
        .unwrap();
    let auth_response = client
        .auth_accept(AuthLoginAcceptRequest {
            auth_token: auth_response.auth_token,
        })
        .await
        .unwrap();
    client.token = Some(auth_response.permissions_token);
    client.claim_burn(claim_burn_request).await.unwrap()
}

pub async fn create_account(world: &mut TariWorld, account_name: String, wallet_daemon_name: String) {
    let key = get_key_manager(world).get_active_key().expect("No active keypair");
    world
        .account_public_keys
        .insert(account_name.clone(), (key.secret_key.clone(), key.public_key.clone()));

    let request = AccountsCreateRequest {
        account_name: Some(account_name.clone()),
        signing_key_index: None,
        custom_access_rules: None,
        is_default: false,
        fee: None,
    };

    let mut client = get_wallet_daemon_client(world, wallet_daemon_name).await;
    let auth_reponse = client
        .auth_request(AuthLoginRequest {
            permissions: JrpcPermissions(vec![JrpcPermission::Admin]),
        })
        .await
        .unwrap();
    let auth_response = client
        .auth_accept(AuthLoginAcceptRequest {
            auth_token: auth_reponse.auth_token,
        })
        .await
        .unwrap();

    client.token = Some(auth_response.permissions_token);
    let resp = client.create_account(request).await.unwrap();

    // store the account component address and other substate addresses for later reference
    add_substate_addresses(world, account_name, resp.result.result.accept().unwrap());
}

pub async fn transfer(
    world: &mut TariWorld,
    account_name: String,
    destination_public_key: RistrettoPublicKey,
    resource_address: ResourceAddress,
    amount: Amount,
    wallet_daemon_name: String,
) -> TransferResponse {
    let mut client = get_wallet_daemon_client(world, wallet_daemon_name).await;

    let account = Some(ComponentAddressOrName::Name(account_name));
    let fee = Some(Amount(1));

    let request = TransferRequest {
        account,
        amount,
        resource_address,
        destination_public_key,
        fee,
    };

    let auth_response = client
        .auth_request(AuthLoginRequest {
            permissions: JrpcPermissions(vec![JrpcPermission::Admin]),
        })
        .await
        .unwrap();
    let auth_response = client
        .auth_accept(AuthLoginAcceptRequest {
            auth_token: auth_response.auth_token,
        })
        .await
        .unwrap();
    client.token = Some(auth_response.permissions_token);

    client.accounts_transfer(request).await.unwrap()
}

pub(crate) async fn get_wallet_daemon_client(world: &TariWorld, wallet_daemon_name: String) -> WalletDaemonClient {
    let port = world.wallet_daemons.get(&wallet_daemon_name).unwrap().json_rpc_port;
    get_walletd_client(port).await
}
