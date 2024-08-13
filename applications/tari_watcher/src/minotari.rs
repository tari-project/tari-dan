// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use anyhow::bail;
use minotari_app_grpc::tari_rpc::TipInfoResponse;
use minotari_app_grpc::tari_rpc::{self as grpc, GetActiveValidatorNodesResponse};
use minotari_node_grpc_client::BaseNodeGrpcClient;
use minotari_wallet_grpc_client::WalletGrpcClient;
use tari_common::exit_codes::ExitCode;
use tari_common::exit_codes::ExitError;
use tonic::Streaming;
use tonic::{transport::Channel, Response};

#[derive(Clone)]
pub struct Minotari {
    bootstrapped: bool,
    node_grpc_address: String,
    wallet_grpc_address: String,
    node: Option<BaseNodeGrpcClient<Channel>>,
    wallet: Option<WalletGrpcClient<Channel>>,
}

impl Minotari {
    pub fn new(node_grpc_address: String, wallet_grpc_address: String) -> Self {
        Self {
            bootstrapped: false,
            node_grpc_address,
            wallet_grpc_address,
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

    pub async fn get_active_validator_nodes(
        &self,
    ) -> anyhow::Result<Response<Streaming<GetActiveValidatorNodesResponse>>> {
        if self.node.is_none() {
            bail!("Node client not connected");
        }

        // could be a good idea to cache this or similar in the future, if perf suffers
        let info = self.node.clone().unwrap().get_tip_info(grpc::Empty {}).await?;
        let block_height = info.into_inner().metadata.unwrap().best_block_height;

        Ok(self
            .node
            .clone()
            .unwrap()
            .get_active_validator_nodes(grpc::GetActiveValidatorNodesRequest {
                height: block_height,
                sidechain_id: vec![],
            })
            .await?)
    }
}
