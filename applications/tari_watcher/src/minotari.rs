// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::path::PathBuf;

use anyhow::{anyhow, bail};
use log::*;
use minotari_app_grpc::tari_rpc::{self as grpc, GetActiveValidatorNodesResponse, RegisterValidatorNodeResponse};
use minotari_node_grpc_client::BaseNodeGrpcClient;
use minotari_wallet_grpc_client::WalletGrpcClient;
use tari_crypto::tari_utilities::ByteArray;
use tari_ootle_common_types::layer_one_transaction::{
    LayerOnePayloadType,
    LayerOneTransactionDef,
    ValidatorExitParams,
    ValidatorRegistrationParams,
};
use tari_transaction_components::transaction_components::{MemoField, memo_field::TxType};
use tonic::transport::Channel;
use url::Url;

use crate::helpers::read_registration_file;

#[derive(Clone)]
pub struct MinotariNodes {
    node_grpc_address: Url,
    wallet_grpc_address: Url,
    node_registration_file: PathBuf,
    current_height: u64,
}

#[derive(Debug, Clone)]
pub struct TipStatus {
    block_height: u64,
}

impl TipStatus {
    pub fn height(&self) -> u64 {
        self.block_height
    }
}

impl MinotariNodes {
    pub fn new(node_grpc_address: Url, wallet_grpc_address: Url, node_registration_file: PathBuf) -> Self {
        Self {
            node_grpc_address,
            wallet_grpc_address,
            node_registration_file,
            current_height: 0,
        }
    }

    async fn connect_wallet(&self) -> anyhow::Result<WalletGrpcClient<Channel>> {
        log::debug!("Connecting to wallet on gRPC {}", self.wallet_grpc_address);
        let client = WalletGrpcClient::connect(self.wallet_grpc_address.as_str()).await?;
        Ok(client)
    }

    async fn connect_node(&self) -> anyhow::Result<BaseNodeGrpcClient<Channel>> {
        debug!("Connecting to base node on gRPC {}", self.node_grpc_address);
        let client = BaseNodeGrpcClient::connect(self.node_grpc_address.to_string()).await?;
        Ok(client)
    }

    pub async fn get_tip_status(&mut self) -> anyhow::Result<TipStatus> {
        let inner = self
            .connect_node()
            .await?
            .get_tip_info(grpc::Empty {})
            .await?
            .into_inner();

        let metadata = inner
            .metadata
            .ok_or_else(|| anyhow!("Base node returned no metadata".to_string()))?;

        self.current_height = metadata.best_block_height;

        Ok(TipStatus {
            block_height: metadata.best_block_height,
        })
    }

    pub async fn get_active_validator_nodes(&self) -> anyhow::Result<Vec<GetActiveValidatorNodesResponse>> {
        let height = self.current_height;
        let mut stream = self
            .connect_node()
            .await?
            .get_active_validator_nodes(grpc::GetActiveValidatorNodesRequest {
                height,
                sidechain_id: vec![],
            })
            .await?
            .into_inner();

        let mut vns = Vec::new();
        loop {
            match stream.message().await {
                Ok(Some(val)) => {
                    vns.push(val);
                },
                Ok(None) => {
                    break;
                },
                Err(e) => {
                    bail!("Error getting active VN: {}", e);
                },
            }
        }

        if vns.is_empty() {
            log::debug!("No active VNs found at height: {}", height);
        }

        Ok(vns)
    }

    pub async fn register_validator_node(&mut self) -> anyhow::Result<RegisterValidatorNodeResponse> {
        info!("Preparing to send a VN registration request");

        let info = read_registration_file(self.node_registration_file.clone())
            .await?
            .ok_or_else(|| {
                anyhow!(
                    "No registration data found in file: {}",
                    self.node_registration_file.display()
                )
            })?;
        let resp = self
            .connect_wallet()
            .await?
            .register_validator_node(grpc::RegisterValidatorNodeRequest {
                validator_node_public_key: info.public_key.to_vec(),
                validator_node_signature: Some(grpc::Signature {
                    public_nonce: info.signature.public_nonce().to_vec(),
                    signature: info.signature.signature().to_vec(),
                }),
                validator_node_claim_public_key: info.claim_fees_public_key.to_vec(),
                max_epoch: 0u64,
                fee_per_gram: 10,
                sidechain_deployment_key: vec![],
                payment_id: MemoField::new_open(
                    format!("VN registration: {}", info.public_key).into_bytes(),
                    TxType::ValidatorNodeRegistration,
                )
                .map_err(|e| anyhow!("Failed to create payment ID: {}", e))?
                .to_bytes(),
            })
            .await?
            .into_inner();
        if !resp.is_success {
            bail!("Failed to register VN: {}", resp.failure_message);
        }

        info!("VN registration request sent successfully");

        Ok(resp)
    }

    pub async fn submit_transaction(
        &mut self,
        transaction_def: LayerOneTransactionDef<serde_json::Value>,
    ) -> anyhow::Result<()> {
        let proof_type = transaction_def.payload_type;
        let mut client = self.connect_wallet().await?;
        match proof_type {
            LayerOnePayloadType::ValidatorRegistration => {
                let registration = serde_json::from_value::<ValidatorRegistrationParams>(transaction_def.payload)?;

                let resp = client
                    .register_validator_node(grpc::RegisterValidatorNodeRequest {
                        validator_node_public_key: registration.public_key.as_bytes().to_vec(),
                        validator_node_signature: Some(grpc::Signature {
                            public_nonce: registration.signature.public_nonce().to_vec(),
                            signature: registration.signature.signature().to_vec(),
                        }),
                        validator_node_claim_public_key: registration.claim_public_key.as_bytes().to_vec(),
                        max_epoch: registration.max_epoch.as_u64(),
                        fee_per_gram: 10,
                        payment_id: MemoField::new_open(
                            format!("VN registration: {}", registration.public_key).into_bytes(),
                            TxType::ValidatorNodeRegistration,
                        )
                        .map_err(|e| anyhow!("Failed to create payment ID: {}", e))?
                        .to_bytes(),
                        sidechain_deployment_key: registration
                            .sidechain_public_key
                            .map(|key| key.to_vec())
                            .unwrap_or_default(),
                    })
                    .await?;

                let resp = resp.into_inner();
                if !resp.is_success {
                    bail!("Failed to register VN: {}", resp.failure_message);
                }
                info!(
                    "{} transaction sent successfully (tx_id={})",
                    proof_type, resp.transaction_id
                );
            },
            LayerOnePayloadType::ValidatorExit => {
                let exit = serde_json::from_value::<ValidatorExitParams>(transaction_def.payload)?;

                let resp = client
                    .submit_validator_node_exit(grpc::SubmitValidatorNodeExitRequest {
                        validator_node_public_key: exit.public_key.as_bytes().to_vec(),
                        validator_node_signature: None,
                        max_epoch: exit.max_epoch.as_u64(),
                        fee_per_gram: 10,
                        message: format!("Validator: Automatically submitted {proof_type} transaction").into_bytes(),
                        // TODO: This will not work as the deployment key is a secret key.
                        sidechain_deployment_key: exit.sidechain_public_key.map(|key| key.to_vec()).unwrap_or_default(),
                    })
                    .await?;

                let resp = resp.into_inner();
                if !resp.is_success {
                    bail!("Failed to register VN: {}", resp.failure_message);
                }
                info!(
                    "{} transaction sent successfully (tx_id={})",
                    proof_type, resp.transaction_id
                );
            },
        };

        Ok(())
    }
}
