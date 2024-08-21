// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use log::*;
use std::path::PathBuf;

use anyhow::bail;
use minotari_app_grpc::tari_rpc::{
    self as grpc, GetActiveValidatorNodesResponse, RegisterValidatorNodeResponse, TipInfoResponse,
};
use minotari_node_grpc_client::BaseNodeGrpcClient;
use minotari_wallet_grpc_client::WalletGrpcClient;
use tari_common::exit_codes::{ExitCode, ExitError};
use tari_crypto::tari_utilities::ByteArray;
use tonic::transport::Channel;

use crate::helpers::{read_registration_file, to_block_height};

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

        let tip_info = self.get_tip_status().await?;
        let height = to_block_height(tip_info);
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

    pub async fn register_validator_node(&self) -> anyhow::Result<RegisterValidatorNodeResponse> {
        if !self.bootstrapped {
            bail!("Node client not connected");
        }

        info!("Preparing to send a node registration request");

        let info = read_registration_file(self.node_registration_file.clone()).await?;
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

        info!("Node registration request sent successfully");

        Ok(resp)
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
}
