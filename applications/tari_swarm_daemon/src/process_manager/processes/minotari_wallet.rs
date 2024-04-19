//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use anyhow::anyhow;
use minotari_node_grpc_client::grpc;
use minotari_wallet_grpc_client::WalletGrpcClient;
use tari_crypto::tari_utilities::ByteArray;

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

    pub async fn register_validator_node(&self, info: ValidatorRegistrationInfo) -> anyhow::Result<()> {
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

        log::info!("🟢 Registered validator node with tx_id: {}", resp.transaction_id);
        Ok(())
    }

    pub async fn get_balance(&self) -> anyhow::Result<grpc::GetBalanceResponse> {
        let mut client = self.connect_client().await?;
        let balance = client.get_balance(grpc::GetBalanceRequest {}).await?.into_inner();
        Ok(balance)
    }
}
