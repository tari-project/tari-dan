//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use anyhow::anyhow;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use minotari_node_grpc_client::grpc;
use minotari_wallet_grpc_client::WalletGrpcClient;
use serde::Serialize;
use serde_json::json;
use tari_common_types::types::{Commitment, PrivateKey};
use tari_crypto::{
    ristretto::{RistrettoComSig, RistrettoPublicKey},
    tari_utilities::ByteArray,
};

use crate::process_manager::{Instance, ValidatorRegistrationInfo};

pub struct MinoTariWalletProcess {
    instance: Instance,
}

impl MinoTariWalletProcess {
    pub fn new(instance: Instance) -> Self {
        Self { instance }
    }

    pub fn instance(&self) -> &Instance {
        &self.instance
    }

    pub fn instance_mut(&mut self) -> &mut Instance {
        &mut self.instance
    }

    pub async fn connect_client(&self) -> anyhow::Result<WalletGrpcClient<tonic::transport::Channel>> {
        let port = self
            .instance
            .allocated_ports()
            .get("grpc")
            .ok_or_else(|| anyhow!("No wallet port allocated"))?;
        let client = WalletGrpcClient::connect(&format!("http://localhost:{}", port)).await?;
        Ok(client)
    }

    pub async fn register_validator_node(&self, info: ValidatorRegistrationInfo) -> anyhow::Result<u64> {
        let mut client = self.connect_client().await?;
        let resp = client
            .register_validator_node(grpc::RegisterValidatorNodeRequest {
                validator_node_public_key: info.public_key.to_vec(),
                validator_node_signature: Some(grpc::Signature {
                    public_nonce: info.signature.signature().get_public_nonce().to_vec(),
                    signature: info.signature.signature().get_signature().to_vec(),
                }),
                validator_node_claim_public_key: info.claim_fees_public_key.to_vec(),
                fee_per_gram: 10,
                message: format!("Validator node registration: {}", info.public_key),
                sidechain_deployment_key: vec![],
            })
            .await?;
        let resp = resp.into_inner();
        if !resp.is_success {
            return Err(anyhow!("Failed to register validator node: {}", resp.failure_message));
        }

        Ok(resp.transaction_id)
    }

    pub async fn burn_funds(
        &self,
        amount: u64,
        claim_public_key: &RistrettoPublicKey,
    ) -> anyhow::Result<BurnClaimProofJson> {
        let mut client = self.connect_client().await?;

        let request = grpc::CreateBurnTransactionRequest {
            amount,
            fee_per_gram: 1,
            message: "Burn funds in swarm".to_string(),
            claim_public_key: claim_public_key.to_vec(),
            sidechain_deployment_key: vec![],
        };
        let resp = client.create_burn_transaction(request).await?;
        let resp = resp.into_inner();
        if !resp.is_success {
            return Err(anyhow!("Failed to burn funds: {}", resp.failure_message));
        }

        let ownership_proof = resp
            .ownership_proof
            .ok_or_else(|| anyhow!("No ownership proof in response"))?;
        let commitment =
            Commitment::from_canonical_bytes(&resp.commitment).map_err(|e| anyhow!("commitment parse error: {e}"))?;

        let ownership_proof = RistrettoComSig::new(
            Commitment::from_canonical_bytes(&ownership_proof.public_nonce)
                .map_err(|e| anyhow!("comsig public_nonce parse error {e}"))?,
            PrivateKey::from_canonical_bytes(&ownership_proof.u).map_err(|e| anyhow!("comsig u parse error {e}"))?,
            PrivateKey::from_canonical_bytes(&ownership_proof.v).map_err(|e| anyhow!("comsig v parse error {e}"))?,
        );

        let reciprocal_claim_public_key = RistrettoPublicKey::from_canonical_bytes(&resp.reciprocal_claim_public_key)
            .map_err(|e| anyhow!("reciprocal_claim_public_key parse error {e}"))?;

        let proof = BurnClaimProofJson {
            tx_id: resp.transaction_id,
            claim_public_key: claim_public_key.clone(),
            claim_proof: json!({
                "commitment": BASE64.encode(commitment.as_bytes()),
                "ownership_proof": {
                    "public_nonce": BASE64.encode(ownership_proof.public_nonce().as_bytes()),
                    "u": BASE64.encode(ownership_proof.u().as_bytes()),
                    "v": BASE64.encode(ownership_proof.v().as_bytes())
                },
                "reciprocal_claim_public_key": BASE64.encode(reciprocal_claim_public_key.as_bytes()),
                "range_proof": BASE64.encode(&resp.range_proof),
            }),
        };

        Ok(proof)
    }

    pub async fn get_balance(&self) -> anyhow::Result<grpc::GetBalanceResponse> {
        let mut client = self.connect_client().await?;
        let balance = client.get_balance(grpc::GetBalanceRequest {}).await?.into_inner();
        Ok(balance)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BurnClaimProofJson {
    pub tx_id: u64,
    pub claim_public_key: RistrettoPublicKey,
    pub claim_proof: serde_json::Value,
}
