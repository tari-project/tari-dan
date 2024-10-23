//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fs,
    fs::File,
    io,
    io::Write,
    path::{Path, PathBuf},
};

use config::Config;
use log::*;
use tari_common::configuration::{ConfigOverrideProvider, Network};

const LOG_TARGET: &str = "tari::application::configuration";

/// Loads the configuration file from the specified path, or creates a new one with the embedded default presets if it
/// does not. This also prompts the user.
pub fn load_configuration<P: AsRef<Path>, TOverride: ConfigOverrideProvider>(
    network_override: Option<Network>,
    config_path: P,
    create_if_not_exists: bool,
    overrides: &TOverride,
) -> Result<Config, ConfigError> {
    debug!(
        target: LOG_TARGET,
        "Loading configuration file from  {}",
        config_path.as_ref().display()
    );
    if !config_path.as_ref().exists() && create_if_not_exists {
        let sources = get_default_config();
        write_config_to(&config_path, &sources)?;
    }

    load_configuration_with_overrides(network_override, config_path, overrides)
}

/// Loads the config at the given path applying all overrides.
pub fn load_configuration_with_overrides<P: AsRef<Path>, TOverride: ConfigOverrideProvider>(
    network_override: Option<Network>,
    config_path: P,
    overrides: &TOverride,
) -> Result<Config, ConfigError> {
    let filename = config_path
        .as_ref()
        .to_str()
        .ok_or_else(|| ConfigError::InvalidConfigPath {
            path: config_path.as_ref().to_path_buf(),
        })?;
    let cfg = Config::builder()
        .add_source(config::File::with_name(filename))
        .add_source(
            config::Environment::with_prefix("TARI")
                .prefix_separator("_")
                .separator("__"),
        )
        .build()?;
    // let network = match cfg.get_string("network") {
    //     Ok(network) => {
    //         Network::from_str(&network).map_err(|e| ConfigError::new("Invalid network", Some(e.to_string())))?
    //     },
    //     Err(config::ConfigError::NotFound(_)) => {
    //         debug!(target: LOG_TARGET, "No network configuration found. Using default network '{}'.",
    // Network::default());         Network::default()
    //     },
    //     Err(e) => {
    //         return Err(ConfigError::new(
    //             "Could not get network configuration",
    //             Some(e.to_string()),
    //         ));
    //     },
    // };

    let network = network_override.unwrap_or_default();

    let overrides = overrides.get_config_property_overrides(&network);
    // if overrides.is_empty() {
    //     return Ok(cfg);
    // }

    let mut cfg = Config::builder().add_source(cfg);
    for (key, value) in overrides {
        cfg = cfg
            .set_override(key.as_str(), value.as_str())
            .map_err(|ce| ConfigError::new("Could not override config property", Some(ce.to_string())))?;
    }
    cfg = cfg
        .set_override("network", network.to_string())
        .map_err(|ce| ConfigError::new("Could not override config property", Some(ce.to_string())))?;
    let cfg = cfg
        .build()
        .map_err(|ce| ConfigError::new("Could not build config", Some(ce.to_string())))?;

    Ok(cfg)
}

/// Returns the default configuration file template in parts from the embedded presets. If use_mining_config is true,
/// the base node configuration that enables mining is returned, otherwise the non-mining configuration is returned.
pub fn get_default_config() -> [&'static str; 5] {
    [
        include_str!("../config_presets/a_common.toml"),
        include_str!("../config_presets/b_peer_seeds.toml"),
        include_str!("../config_presets/c_validator_node.toml"),
        include_str!("../config_presets/d_indexer.toml"),
        include_str!("../config_presets/e_dan_wallet_daemon.toml"),
    ]
}

/// Writes a single file concatenating all the provided sources to the specified path. If the parent directory does not
/// exist, it is created. If the file already exists, it is overwritten.
pub fn write_config_to<P: AsRef<Path>>(path: P, sources: &[&str]) -> Result<(), std::io::Error> {
    if let Some(d) = path.as_ref().parent() {
        fs::create_dir_all(d)?
    };
    let mut file = File::create(path)?;
    for source in sources {
        file.write_all(source.trim().as_bytes())?;
        file.write_all(b"\n\n")?;
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    IoError(#[from] io::Error),
    #[error("Invalid config file path '{}'", .path.display())]
    InvalidConfigPath { path: PathBuf },
    #[error(transparent)]
    ConfigError(#[from] config::ConfigError),
    #[error("{cause}{}", .error.as_ref().map(|s| format!(": ({})", s)).unwrap_or_default())]
    CustomError { cause: &'static str, error: Option<String> },
}

impl ConfigError {
    pub fn new(cause: &'static str, source: Option<String>) -> Self {
        Self::CustomError { cause, error: source }
    }
}
