//    Copyright 2026 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use log::*;
use ootle_network::Network;
use tari_common::exit_codes::{ExitCode, ExitError};
use tari_engine_types::{Epoch, ProtocolVersion, UnknownProtocolVersionError};
use tari_ootle_storage::global::{GlobalDb, GlobalDbAdapter, MetadataKey};

const LOG_TARGET: &str = "tari::ootle::protocol_activation";

/// Refuses to start when this binary's schema activation schedule disagrees with the one the node
/// has already run under, and otherwise records the schedule it started with.
///
/// Shared by every application that derives substate value hashes — a validator node diverges from
/// consensus, an indexer fails to verify the proofs it serves, and neither surfaces the cause at the
/// point it goes wrong. See [`ProtocolVersion::check_activation_schedule`] for what is compared and
/// why.
///
/// `allow_past_activation` is for a node whose state is being discarded; on a node with live state
/// it converts a caught error into the divergence the check exists to prevent.
pub fn check_and_record_activation_schedule<TAdapter>(
    network: Network,
    global_db: &GlobalDb<TAdapter>,
    allow_past_activation: bool,
) -> Result<(), ExitError>
where
    TAdapter: GlobalDbAdapter,
    TAdapter::Error: std::fmt::Display,
{
    let db_error = |e: TAdapter::Error| ExitError::new(ExitCode::DatabaseError, e);

    let mut tx = global_db.create_transaction().map_err(db_error)?;
    let mut metadata = global_db.metadata(&mut tx);
    let recorded: Vec<Epoch> = metadata
        .get_metadata(MetadataKey::ProtocolActivationSchedule.as_key_bytes())
        .map_err(db_error)?
        .unwrap_or_default();
    let last_known_epoch: Option<Epoch> = metadata
        .get_metadata(MetadataKey::EpochManagerCurrentEpoch.as_key_bytes())
        .map_err(db_error)?;
    // Read as the raw number so that a version this binary does not know is reported as a node to resync rather
    // than as an undecodable database.
    let recorded_genesis: Option<u32> = metadata
        .get_metadata(MetadataKey::ProtocolGenesisVersion.as_key_bytes())
        .map_err(db_error)?;
    let recorded_genesis = match recorded_genesis.map(ProtocolVersion::try_from).transpose() {
        Ok(version) => version,
        Err(UnknownProtocolVersionError(version)) if allow_past_activation => {
            warn!(
                target: LOG_TARGET,
                "⚠️ This node has run under protocol version {version}, which this binary does not support. \
                 Continuing because allow_past_protocol_activation is set."
            );
            None
        },
        Err(UnknownProtocolVersionError(version)) => {
            let err = format!(
                "This node has run under protocol version {version}, which this binary does not support. Starting \
                 would hash every block under a version its stored state was not hashed under. Delete the node's data \
                 directory and resync."
            );
            error!(target: LOG_TARGET, "🛑 {err}");
            return Err(ExitError::new(ExitCode::DbInconsistentState, err));
        },
    };

    let genesis = ProtocolVersion::genesis(network);
    let to_record = match ProtocolVersion::check_activation_schedule(
        network,
        &recorded,
        recorded_genesis.unwrap_or(genesis),
        last_known_epoch,
    ) {
        Ok(schedule) => schedule,
        Err(err) if allow_past_activation => {
            warn!(target: LOG_TARGET, "⚠️ {err} Continuing because allow_past_protocol_activation is set.");
            ProtocolVersion::scheduled_activation_epochs(network)
        },
        Err(err) => {
            error!(target: LOG_TARGET, "🛑 {err}");
            return Err(ExitError::new(ExitCode::DbInconsistentState, err));
        },
    };

    if to_record != recorded {
        info!(target: LOG_TARGET, "Recording protocol schema activation schedule: {to_record:?}");
        metadata
            .set_metadata(MetadataKey::ProtocolActivationSchedule.as_key_bytes(), &to_record)
            .map_err(db_error)?;
    }
    if recorded_genesis != Some(genesis) {
        info!(target: LOG_TARGET, "Recording genesis protocol version: {genesis}");
        metadata
            .set_metadata(MetadataKey::ProtocolGenesisVersion.as_key_bytes(), &genesis)
            .map_err(db_error)?;
    }
    global_db.commit(tx).map_err(db_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tari_ootle_storage::global::DbFactory;
    use tari_ootle_storage_sqlite::{SqliteDbFactory, global::SqliteGlobalDbAdapter};
    use tari_template_lib::types::crypto::RistrettoPublicKeyBytes;
    use tempfile::TempDir;

    use super::*;

    type Adapter = SqliteGlobalDbAdapter<RistrettoPublicKeyBytes>;

    fn open_db(dir: &TempDir) -> GlobalDb<Adapter> {
        let factory = SqliteDbFactory::<RistrettoPublicKeyBytes>::new(dir.path().join("global.sqlite"));
        factory.migrate().unwrap();
        factory.get_or_create_global_db().unwrap()
    }

    /// A node that has reached `epoch` with `genesis_version` recorded as its genesis protocol version.
    fn node_with_history(dir: &TempDir, genesis_version: u32, epoch: Epoch) -> GlobalDb<Adapter> {
        let global_db = open_db(dir);
        let mut tx = global_db.create_transaction().unwrap();
        let mut metadata = global_db.metadata(&mut tx);
        metadata
            .set_metadata(MetadataKey::ProtocolGenesisVersion.as_key_bytes(), &genesis_version)
            .unwrap();
        metadata
            .set_metadata(MetadataKey::EpochManagerCurrentEpoch.as_key_bytes(), &epoch)
            .unwrap();
        global_db.commit(tx).unwrap();
        global_db
    }

    fn recorded_genesis(global_db: &GlobalDb<Adapter>) -> Option<ProtocolVersion> {
        let mut tx = global_db.create_transaction().unwrap();
        global_db
            .metadata(&mut tx)
            .get_metadata(MetadataKey::ProtocolGenesisVersion.as_key_bytes())
            .unwrap()
    }

    #[test]
    fn a_new_node_records_the_genesis_version() {
        let dir = tempfile::tempdir().unwrap();
        let global_db = open_db(&dir);
        check_and_record_activation_schedule(Network::LocalNet, &global_db, false).unwrap();
        assert_eq!(
            recorded_genesis(&global_db),
            Some(ProtocolVersion::genesis(Network::LocalNet))
        );
    }

    #[test]
    fn a_node_that_ran_under_an_unknown_version_is_told_to_resync() {
        let dir = tempfile::tempdir().unwrap();
        let global_db = node_with_history(&dir, 3, Epoch(5));
        let err = check_and_record_activation_schedule(Network::LocalNet, &global_db, false).unwrap_err();
        assert!(matches!(err.exit_code, ExitCode::DbInconsistentState), "{err:?}");
        assert!(
            err.details.as_deref().is_some_and(|d| d.contains("protocol version 3")),
            "{err:?}"
        );
    }

    #[test]
    fn allowing_a_past_activation_records_over_an_unknown_version() {
        let dir = tempfile::tempdir().unwrap();
        let global_db = node_with_history(&dir, 3, Epoch(5));
        check_and_record_activation_schedule(Network::LocalNet, &global_db, true).unwrap();
        assert_eq!(
            recorded_genesis(&global_db),
            Some(ProtocolVersion::genesis(Network::LocalNet))
        );
    }
}
