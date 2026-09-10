//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use log::warn;
use serde::{Deserialize, Serialize};
use tari_consensus::consensus_constants::ConsensusConstants;
use tari_engine_types::fees::{ExhaustBurnRate, MAX_EXHAUST_BURN_RATE_BPS};
use tari_ootle_transaction::Network;

const LOG_TARGET: &str = "tari::ootle::consensus_constants_file";

/// Overrides for a LocalNet's consensus constants, applied over the network's built-in values.
///
/// Consensus constants have to agree across a network, so this is authored once and handed to every
/// node on a devnet rather than set per node. Every field is optional: an absent field keeps the
/// network's value, and an absent file leaves the network untouched.
///
/// `num_preshards` is deliberately not settable. Shard identity is derived from it everywhere, so
/// nodes that disagree on it do not form a misconfigured network, they form no network at all.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ConsensusConstantsFile {
    pub base_layer_confirmations: Option<u64>,
    pub committee_size_per_shard_group: Option<u32>,
    pub pacemaker_block_time_secs: Option<u64>,
    pub missed_proposal_suspend_threshold: Option<u64>,
    pub missed_proposal_evict_threshold: Option<u64>,
    pub missed_proposal_recovery_threshold: Option<u64>,
    pub max_block_weight: Option<u64>,
    pub max_commands_in_block: Option<usize>,
    pub max_block_validation_weight: Option<u64>,
    pub max_transaction_weight: Option<u64>,
    pub max_transaction_size_bytes: Option<usize>,
    pub max_block_execution_points: Option<u64>,
    pub max_block_validation_execution_points: Option<u64>,
    pub exhaust_burn_rate_bps: Option<u16>,
    pub max_transaction_validity_epochs: Option<u64>,
    /// Accepted and ignored, so that an existing constants file that sets it still loads:
    /// `deny_unknown_fields` would otherwise reject the whole file.
    #[serde(skip_serializing)]
    pub epoch_end_spread_blocks: Option<u64>,
}

impl ConsensusConstantsFile {
    pub fn apply_to(self, constants: &mut ConsensusConstants) -> Result<(), ConsensusConstantsFileError> {
        macro_rules! set {
            ($($field:ident),+ $(,)?) => {
                $(if let Some(value) = self.$field {
                    constants.$field = value;
                })+
            };
        }

        set!(
            base_layer_confirmations,
            committee_size_per_shard_group,
            missed_proposal_suspend_threshold,
            missed_proposal_evict_threshold,
            missed_proposal_recovery_threshold,
            max_block_weight,
            max_commands_in_block,
            max_block_validation_weight,
            max_transaction_weight,
            max_transaction_size_bytes,
            max_block_execution_points,
            max_block_validation_execution_points,
            max_transaction_validity_epochs,
        );

        if self.epoch_end_spread_blocks.is_some() {
            warn!(
                target: LOG_TARGET,
                "⚠️ 'epoch_end_spread_blocks' in the consensus constants file is ignored and can be removed."
            );
        }

        if let Some(secs) = self.pacemaker_block_time_secs {
            constants.pacemaker_block_time = Duration::from_secs(secs);
        }

        if constants.committee_size_per_shard_group == 0 {
            return Err(ConsensusConstantsFileError::CommitteeSizeIsZero);
        }

        if let Some(bps) = self.exhaust_burn_rate_bps {
            if bps > MAX_EXHAUST_BURN_RATE_BPS {
                return Err(ConsensusConstantsFileError::ExhaustBurnRateTooHigh {
                    bps,
                    max: MAX_EXHAUST_BURN_RATE_BPS,
                });
            }
            constants.exhaust_burn_rate = ExhaustBurnRate::new(bps);
        }

        Ok(())
    }
}

/// Loads the consensus constants for `network`.
///
/// Only a LocalNet reads a file at all. Every other network's constants are part of what its nodes
/// agree on, so there is nothing a local file could say about them that would be safe to act on, and
/// a file left behind by a devnet cannot reach a node that is not on one.
///
/// A `configured` path is one an operator named, so its absence is a misconfiguration and is
/// reported. `default_path` is a convenience - a file may be dropped there to take effect, and its
/// absence is the ordinary case.
pub fn load_consensus_constants(
    network: Network,
    configured: Option<&Path>,
    default_path: &Path,
) -> Result<ConsensusConstants, ConsensusConstantsFileError> {
    let mut constants = ConsensusConstants::from(network);
    if !matches!(network, Network::LocalNet) {
        return Ok(constants);
    }

    let path = match configured {
        Some(path) if !path.exists() => {
            return Err(ConsensusConstantsFileError::ConfiguredFileNotFound {
                path: path.to_path_buf(),
            });
        },
        Some(path) => path,
        None if default_path.exists() => default_path,
        None => return Ok(constants),
    };

    let contents = fs::read_to_string(path).map_err(|source| ConsensusConstantsFileError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let overrides: ConsensusConstantsFile =
        toml::from_str(&contents).map_err(|source| ConsensusConstantsFileError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
    overrides.apply_to(&mut constants)?;
    Ok(constants)
}

#[derive(Debug, thiserror::Error)]
pub enum ConsensusConstantsFileError {
    #[error("Consensus constants file {path} is configured but does not exist")]
    ConfiguredFileNotFound { path: PathBuf },
    #[error("Failed to read consensus constants file {path}: {source}")]
    Read { path: PathBuf, source: std::io::Error },
    #[error("Failed to parse consensus constants file {path}: {source}")]
    Parse { path: PathBuf, source: toml::de::Error },
    #[error("Committee size must be greater than zero")]
    CommitteeSizeIsZero,
    #[error("Exhaust burn rate {bps}bps is above the maximum of {max}bps")]
    ExhaustBurnRateTooHigh { bps: u16, max: u16 },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(toml_str: &str) -> ConsensusConstants {
        let mut constants = ConsensusConstants::from(Network::LocalNet);
        toml::from_str::<ConsensusConstantsFile>(toml_str)
            .unwrap()
            .apply_to(&mut constants)
            .unwrap();
        constants
    }

    #[test]
    fn an_empty_file_keeps_every_network_value() {
        let defaults = ConsensusConstants::from(Network::LocalNet);
        let constants = parse("");
        assert_eq!(
            constants.committee_size_per_shard_group,
            defaults.committee_size_per_shard_group
        );
        assert_eq!(constants.pacemaker_block_time, defaults.pacemaker_block_time);
        assert_eq!(constants.max_block_weight, defaults.max_block_weight);
    }

    #[test]
    fn a_field_that_is_set_replaces_only_itself() {
        let defaults = ConsensusConstants::from(Network::LocalNet);
        let constants = parse("committee_size_per_shard_group = 3");
        assert_eq!(constants.committee_size_per_shard_group, 3);
        assert_eq!(constants.max_block_weight, defaults.max_block_weight);
        assert_eq!(constants.num_preshards, defaults.num_preshards);
    }

    #[test]
    fn a_committee_size_of_zero_is_refused() {
        let mut constants = ConsensusConstants::from(Network::LocalNet);
        let overrides = ConsensusConstantsFile {
            committee_size_per_shard_group: Some(0),
            ..Default::default()
        };
        assert!(matches!(
            overrides.apply_to(&mut constants),
            Err(ConsensusConstantsFileError::CommitteeSizeIsZero)
        ));
    }

    #[test]
    fn a_burn_rate_above_the_maximum_is_refused() {
        let mut constants = ConsensusConstants::from(Network::LocalNet);
        let overrides = ConsensusConstantsFile {
            exhaust_burn_rate_bps: Some(MAX_EXHAUST_BURN_RATE_BPS + 1),
            ..Default::default()
        };
        assert!(matches!(
            overrides.apply_to(&mut constants),
            Err(ConsensusConstantsFileError::ExhaustBurnRateTooHigh { .. })
        ));
    }

    fn write_constants_file(dir: &tempfile::TempDir) -> PathBuf {
        let path = dir.path().join("consensus_constants.toml");
        fs::write(&path, "committee_size_per_shard_group = 3").unwrap();
        path
    }

    #[test]
    fn a_network_that_is_not_local_reads_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_constants_file(&dir);

        for network in [Network::MainNet, Network::Esmeralda, Network::StageNet] {
            let constants = load_consensus_constants(network, Some(&path), &path).unwrap();
            assert_eq!(
                constants.committee_size_per_shard_group,
                ConsensusConstants::from(network).committee_size_per_shard_group
            );
        }
    }

    #[test]
    fn a_file_at_the_default_location_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_constants_file(&dir);

        let constants = load_consensus_constants(Network::LocalNet, None, &path).unwrap();
        assert_eq!(constants.committee_size_per_shard_group, 3);
    }

    #[test]
    fn nothing_at_the_default_location_is_the_ordinary_case() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("consensus_constants.toml");

        let constants = load_consensus_constants(Network::LocalNet, None, &missing).unwrap();
        assert_eq!(
            constants.committee_size_per_shard_group,
            ConsensusConstants::DEFAULT_DEVNET_COMMITTEE_SIZE
        );
    }

    #[test]
    fn a_configured_file_that_is_not_there_is_a_misconfiguration() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("elsewhere.toml");

        assert!(matches!(
            load_consensus_constants(Network::LocalNet, Some(&missing), &missing),
            Err(ConsensusConstantsFileError::ConfiguredFileNotFound { .. })
        ));
    }

    #[test]
    fn a_field_the_network_must_agree_on_by_construction_is_not_settable() {
        let err = toml::from_str::<ConsensusConstantsFile>("num_preshards = 4").unwrap_err();
        assert!(err.to_string().contains("num_preshards"), "{err}");
    }
}
