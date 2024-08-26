// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    fmt::{self, Display},
    path::PathBuf,
};

use tokio::io::{self, AsyncWriteExt};

use crate::{
    constants::{
        DEFAULT_BASE_NODE_GRPC_ADDRESS,
        DEFAULT_BASE_WALLET_GRPC_ADDRESS,
        DEFAULT_MINOTARI_MINER_BINARY_PATH,
        DEFAULT_VALIDATOR_NODE_BINARY_PATH,
    },
    Cli,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Config {
    /// Allow watcher to submit a new validator node registration transaction initially and before
    /// the current registration expires
    pub auto_register: bool,

    /// The Minotari node gRPC address
    pub base_node_grpc_address: String,

    /// The Minotari console wallet gRPC address
    pub base_wallet_grpc_address: String,

    /// The base directory of the watcher with configuration and data files
    pub base_dir: PathBuf,

    /// The path of the validator node registration file, containing signed information required to
    /// submit a registration transaction on behalf of the node
    pub vn_registration_file: PathBuf,

    /// The sidechain ID to use. If not provided, the default Tari sidechain ID will be used.
    pub sidechain_id: Option<String>,

    /// The configuration for managing one or multiple processes
    pub instance_config: Vec<InstanceConfig>,

    /// The process specific configuration for the executables
    pub executable_config: Vec<ExecutableConfig>,

    /// The channel configuration for alerting and monitoring
    pub channel_config: Vec<ChannelConfig>,
}

impl Config {
    pub(crate) async fn write<W: io::AsyncWrite + Unpin>(&self, mut writer: W) -> anyhow::Result<()> {
        let toml = toml::to_string_pretty(self)?;
        writer.write_all(toml.as_bytes()).await?;
        Ok(())
    }

    pub fn missing_conf(&self) -> Option<Vec<&str>> {
        let mut v: Vec<&str> = Vec::new();
        if self.base_node_grpc_address.is_empty() {
            v.push("base_node_grpc_address");
        }
        if self.base_wallet_grpc_address.is_empty() {
            v.push("base_wallet_grpc_address");
        }
        if v.is_empty() {
            None
        } else {
            Some(v)
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum InstanceType {
    TariValidatorNode,
    MinoTariConsoleWallet,
}

impl Display for InstanceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChannelConfig {
    pub name: String,
    pub enabled: bool,
    pub credentials: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutableConfig {
    pub instance_type: InstanceType,
    pub executable_path: Option<PathBuf>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InstanceConfig {
    pub name: String,
    pub instance_type: InstanceType,
    pub num_instances: u32,
    #[serde(alias = "extra_args")]
    pub settings: HashMap<String, String>,
}

impl InstanceConfig {
    pub fn new(instance_type: InstanceType) -> Self {
        Self {
            name: instance_type.to_string(),
            instance_type,
            num_instances: 1,
            settings: HashMap::new(),
        }
    }

    pub fn with_name<S: Into<String>>(mut self, name: S) -> Self {
        self.name = name.into();
        self
    }

    pub fn with_num_instances(mut self, num_instances: u32) -> Self {
        self.num_instances = num_instances;
        self
    }
}

pub fn get_base_config(cli: &Cli) -> anyhow::Result<Config> {
    let executables = vec![
        ExecutableConfig {
            instance_type: InstanceType::TariValidatorNode,
            executable_path: Some(DEFAULT_VALIDATOR_NODE_BINARY_PATH.into()),
            env: vec![],
        },
        ExecutableConfig {
            instance_type: InstanceType::MinoTariConsoleWallet,
            executable_path: Some(DEFAULT_MINOTARI_MINER_BINARY_PATH.into()),
            env: vec![],
        },
    ];
    let instances = [
        InstanceConfig::new(InstanceType::TariValidatorNode)
            .with_name("tari_validator_node")
            .with_num_instances(1),
        InstanceConfig::new(InstanceType::MinoTariConsoleWallet)
            .with_name("minotari_wallet")
            .with_num_instances(1),
    ];

    let base_dir = cli.common.base_dir.clone();
    let vn_registration_file = base_dir
        .join("data")
        .join("vn1")
        .join("esmeralda")
        .join("registration.json");

    Ok(Config {
        auto_register: true,
        // must contain protocol and port
        base_node_grpc_address: DEFAULT_BASE_NODE_GRPC_ADDRESS.to_string(),
        base_wallet_grpc_address: DEFAULT_BASE_WALLET_GRPC_ADDRESS.to_string(),
        base_dir: base_dir.clone(),
        sidechain_id: None,
        vn_registration_file,
        instance_config: instances.to_vec(),
        executable_config: executables,
        channel_config: vec![
            ChannelConfig {
                name: "mattermost".to_string(),
                enabled: true,
                credentials: "".to_string(),
            },
            ChannelConfig {
                name: "telegram".to_string(),
                enabled: true,
                credentials: "".to_string(),
            },
        ],
    })
}
