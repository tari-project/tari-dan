//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt,
    fmt::Display,
    fs::File,
    io,
    path::{Path, PathBuf},
};

use crate::cli::Cli;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Config {
    pub base_dir: PathBuf,
    pub webserver: WebserverConfig,
    #[serde(flatten)]
    pub processes: ProcessesConfig,
}

impl Config {
    pub fn load_with_cli(cli: &Cli) -> anyhow::Result<Self> {
        let mut config = Self::load_from_file(cli.get_config_path())?;
        config.overrides_from_cli(cli);
        Ok(config)
    }

    pub fn load_from_file<P: AsRef<Path>>(file: P) -> anyhow::Result<Self> {
        let mut file = File::open(file)?;
        Self::load_from_reader(&mut file)
    }

    pub fn load_from_reader<R: io::Read>(reader: &mut R) -> anyhow::Result<Self> {
        let mut s = String::new();
        reader.read_to_string(&mut s)?;
        let config = toml::from_str(&s)?;
        Ok(config)
    }

    pub(crate) fn write<W: io::Write>(&self, mut writer: W) -> anyhow::Result<()> {
        let toml = toml::to_string_pretty(self)?;
        writer.write_all(toml.as_bytes())?;
        Ok(())
    }

    fn overrides_from_cli(&mut self, cli: &Cli) {
        if let Some(ref base_dir) = cli.common.base_dir {
            self.base_dir = base_dir.clone();
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WebserverConfig {
    pub bind_address: String,
}

impl Default for WebserverConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:8080".to_string(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProcessesConfig {
    pub always_compile: bool,
    pub instances: Vec<InstanceConfig>,
    pub executables: Vec<ExecutableConfig>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InstanceConfig {
    pub name: String,
    pub instance_type: InstanceType,
    pub num_instances: u32,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum InstanceType {
    MinoTariNode,
    MinoTariConsoleWallet,
    MinoTariMiner,
    TariValidatorNode,
    TariIndexer,
    TariWalletDaemon,
}

impl Display for InstanceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutableConfig {
    pub instance_type: InstanceType,
    pub execuable_path: Option<PathBuf>,
    pub compile: Option<CompileConfig>,
    pub env: Vec<(String, String)>,
}

impl ExecutableConfig {
    pub fn get_executable_path(&self) -> Option<PathBuf> {
        self.execuable_path
            .clone()
            .or_else(|| self.compile.as_ref().map(|c| c.target_dir().join(&c.package_name)))
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompileConfig {
    pub working_dir: Option<PathBuf>,
    pub package_name: String,
    pub target_dir: Option<PathBuf>,
}

impl CompileConfig {
    pub fn target_dir(&self) -> PathBuf {
        self.target_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("target/release"))
    }

    pub fn working_dir(&self) -> PathBuf {
        self.working_dir.clone().unwrap_or_else(|| PathBuf::from("."))
    }
}
