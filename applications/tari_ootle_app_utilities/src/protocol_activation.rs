//    Copyright 2026 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use log::*;
use ootle_network::Network;
use tari_common::exit_codes::{ExitCode, ExitError};
use tari_engine_types::{Epoch, ProtocolVersion};
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
    // Absent means V0: every binary predating this key launched every network at V0.
    let recorded_genesis: ProtocolVersion = metadata
        .get_metadata(MetadataKey::ProtocolGenesisVersion.as_key_bytes())
        .map_err(db_error)?
        .unwrap_or(ProtocolVersion::V0);

    let genesis = ProtocolVersion::genesis(network);
    let to_record =
        match ProtocolVersion::check_activation_schedule(network, &recorded, recorded_genesis, last_known_epoch) {
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
    if genesis != recorded_genesis {
        info!(target: LOG_TARGET, "Recording genesis protocol version: {genesis}");
        metadata
            .set_metadata(MetadataKey::ProtocolGenesisVersion.as_key_bytes(), &genesis)
            .map_err(db_error)?;
    }
    global_db.commit(tx).map_err(db_error)?;
    Ok(())
}
