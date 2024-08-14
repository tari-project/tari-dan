// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use anyhow::bail;
use minotari_app_grpc::tari_rpc::{self as grpc, GetActiveValidatorNodesResponse, TipInfoResponse};
use minotari_node_grpc_client::BaseNodeGrpcClient;
use minotari_wallet_grpc_client::WalletGrpcClient;
use std::path::PathBuf;
use tari_common::exit_codes::{ExitCode, ExitError};
use tari_common_types::types::PublicKey;
use tari_core::transactions::transaction_components::ValidatorNodeSignature;
use tari_crypto::tari_utilities::ByteArray;
use tokio::fs;
use tonic::transport::Channel;

#[derive(Clone)]
pub struct Minotari {
    bootstrapped: bool,
    node_grpc_address: String,
    wallet_grpc_address: String,
    node_registration_file: PathBuf,
    node: Option<BaseNodeGrpcClient<Channel>>,
    wallet: Option<WalletGrpcClient<Channel>>,
}

impl Minotari {
    pub fn new(node_grpc_address: String, wallet_grpc_address: String, node_registration_file: PathBuf) -> Self {
        Self {
            bootstrapped: false,
            node_grpc_address,
            wallet_grpc_address,
            node_registration_file,
            node: None,
            wallet: None,
        }
    }

    pub async fn bootstrap(&mut self) -> anyhow::Result<()> {
        if self.bootstrapped {
            return Ok(());
        }

        self.connect_node().await?;
        self.connect_wallet().await?;
        self.bootstrapped = true;
        Ok(())
    }

    async fn connect_wallet(&mut self) -> anyhow::Result<()> {
        log::info!("Connecting to wallet on gRPC {}", self.wallet_grpc_address.clone());
        let client = WalletGrpcClient::connect(&self.wallet_grpc_address).await?;

        self.wallet = Some(client);
        Ok(())
    }

    async fn connect_node(&mut self) -> anyhow::Result<()> {
        log::info!("Connecting to base node on gRPC {}", self.node_grpc_address.clone());
        let client = BaseNodeGrpcClient::connect(self.node_grpc_address.clone())
            .await
            .map_err(|e| ExitError::new(ExitCode::ConfigError, e))?;

        self.node = Some(client);

        Ok(())
    }

    pub async fn get_tip_status(&self) -> anyhow::Result<TipInfoResponse> {
        if !self.bootstrapped {
            bail!("Node client not connected");
        }

        Ok(self
            .node
            .clone()
            .unwrap()
            .get_tip_info(grpc::Empty {})
            .await?
            .into_inner())
    }

    pub async fn get_active_validator_nodes(&self) -> anyhow::Result<Vec<GetActiveValidatorNodesResponse>> {
        if !self.bootstrapped {
            bail!("Node client not connected");
        }

        let height = self.get_block_height().await?;
        let mut stream = self
            .node
            .clone()
            .unwrap()
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
                    bail!("Error getting active validator nodes: {}", e);
                },
            }
        }

        if vns.is_empty() {
            log::debug!("No active validator nodes found at height: {}", height);
        }

        Ok(vns)
    }

    pub async fn register_validator_node(&self) -> anyhow::Result<u64> {
        if !self.bootstrapped {
            bail!("Node client not connected");
        }

        let info = get_registration_info(self.node_registration_file.clone()).await?;
        let sig = info.signature.signature();
        let resp = self
            .wallet
            .clone()
            .unwrap()
            .register_validator_node(grpc::RegisterValidatorNodeRequest {
                validator_node_public_key: info.public_key.to_vec(),
                validator_node_signature: Some(grpc::Signature {
                    public_nonce: sig.get_public_nonce().to_vec(),
                    signature: sig.get_signature().to_vec(),
                }),
                validator_node_claim_public_key: info.claim_fees_public_key.to_vec(),
                fee_per_gram: 10,
                message: format!("Validator node registration: {}", info.public_key),
                sidechain_deployment_key: vec![],
            })
            .await?
            .into_inner();
        if !resp.is_success {
            bail!("Failed to register validator node: {}", resp.failure_message);
        }

        Ok(resp.transaction_id)
    }

    pub async fn get_validator_expiration(&self) -> anyhow::Result<ValidatorExpirationInfo> {
        if !self.bootstrapped {
            bail!("Node client not connected");
        }

        let height = self.get_block_height().await?;
        let constants = self.get_consensus_constants(height).await?;

        Ok(ValidatorExpirationInfo {
            validator_node_validity_period: constants.validator_node_validity_period,
            epoch_length: constants.epoch_length,
        })
    }

    pub async fn get_consensus_constants(&self, block_height: u64) -> anyhow::Result<grpc::ConsensusConstants> {
        if !self.bootstrapped {
            bail!("Node client not connected");
        }

        let constants = self
            .node
            .clone()
            .unwrap()
            .get_constants(grpc::BlockHeight { block_height })
            .await?
            .into_inner();

        Ok(constants)
    }

    async fn get_block_height(&self) -> anyhow::Result<u64> {
        Ok(self.get_tip_status().await?.metadata.unwrap().best_block_height)
    }
}

pub struct ValidatorExpirationInfo {
    pub validator_node_validity_period: u64,
    pub epoch_length: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ValidatorNodeRegistration {
    pub signature: ValidatorNodeSignature,
    pub public_key: PublicKey,
    pub claim_fees_public_key: PublicKey,
}

async fn get_registration_info(vn_registration_file: PathBuf) -> anyhow::Result<ValidatorNodeRegistration> {
    log::debug!(
        "Using VN registration file at: {}",
        vn_registration_file.clone().into_os_string().into_string().unwrap()
    );

    let info = fs::read_to_string(vn_registration_file).await?;
    let reg = json5::from_str(&info)?;
    Ok(reg)
}
