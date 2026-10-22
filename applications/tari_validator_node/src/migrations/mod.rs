//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Database state migrations.
//!
//! Migrations upgrade the persisted state of *already-running* nodes when a change would otherwise
//! only take effect for freshly bootstrapped databases. Each database records the schema version it
//! was last brought up to ([`DatabaseMigrationVersion`]). [`migrate`] upgrades a stored version for which a
//! migration step exists and refuses any other version below [`CURRENT_VERSION`]. A fresh database skips migrations
//! entirely - it is stamped directly with `CURRENT_VERSION` once the genesis state is laid down.
//!
//! # Adding a migration
//!
//! To take the schema from version `N` to `N + 1`, add `v{N+1}.rs` with `pub fn migrate(...)`, bump
//! `CURRENT_SCHEMA_VERSION` (in the state store, beside `DatabaseMigrationVersion`), and step the stored version up
//! one migration at a time in [`migrate`], persisting each version it reaches:
//!
//! ```ignore
//! while version < CURRENT_VERSION {
//!     match version {
//!         0 => v1::migrate(tx)?,
//!         other => anyhow::bail!("no migration defined for database version {other}"),
//!     }
//!     version += 1;
//!     tx.db().cf(DatabaseMigrationVersion)?.put(&ByteColumn, &version, OPERATION)?;
//! }
//! ```
//!
//! A migration must be able to run against a database at any earlier supported version, so it may not assume the
//! current schema of anything it does not itself write.
//!
//! IMPORTANT: a migration that creates or mutates substates must write them to the per-shard state
//! tree (JMT), not only the substate store - otherwise they have no inclusion proof and verified
//! reads of them fail. Mirror [`crate::genesis_state::create_genesis_state`], which commits each
//! substate to both the store and the state tree. (Note that, unlike genesis, adding state-tree
//! entries to a live chain shifts its state root, so such a migration is itself consensus-affecting.)

use log::*;
use tari_consensus::consensus_constants::ConsensusConstants;
use tari_ootle_app_utilities::genesis_governance::GenesisCouncil;
use tari_ootle_common_types::{NodeAddressable, optional::Optional};
use tari_ootle_transaction::Network;
use tari_state_store_rocksdb::{
    codecs::ByteColumn,
    // The version constant lives in the state store because it describes the on-disk schema, and tools that write to
    // a database directly must check it before doing so. Bump it there and apply the upgrade in `migrate`.
    column_families::bookkeeping::{CURRENT_SCHEMA_VERSION as CURRENT_VERSION, DatabaseMigrationVersion},
    writer::RocksDbStateStoreWriteTransaction,
};

use crate::genesis_state::create_genesis_state;

const LOG_TARGET: &str = "tari::validator_node::migrations";

pub fn migrate<TAddr: NodeAddressable + 'static>(
    tx: &mut RocksDbStateStoreWriteTransaction<'_, TAddr>,
    network: Network,
    consensus_constants: &ConsensusConstants,
    genesis_council: &GenesisCouncil,
) -> anyhow::Result<()> {
    const OPERATION: &str = "migrate";

    let maybe_version = {
        let db = tx.db();
        db.cf(DatabaseMigrationVersion)?
            .get(&ByteColumn, OPERATION)
            .optional()?
    };

    match maybe_version {
        Some(version) if version == CURRENT_VERSION => {
            debug!(
                target: LOG_TARGET,
                "Database already bootstrapped at migration version {version} (current {CURRENT_VERSION})"
            );
        },
        Some(version) if version > CURRENT_VERSION => {
            anyhow::bail!(
                "Database schema version {version} is newer than this binary's {CURRENT_VERSION}. Upgrade the binary, \
                 or if a release incompatible with this network wrote the database, delete the validator's data \
                 directory and resync."
            );
        },
        Some(version) => {
            anyhow::bail!(
                "Database schema version {version} is older than this binary's {CURRENT_VERSION}, and no migration \
                 upgrades it. Delete the validator's data directory and resync."
            );
        },
        // A fresh database: lay down the genesis state and stamp the current version.
        None => {
            info!(target: LOG_TARGET, "🌱 Fresh database - adding genesis state");
            create_genesis_state(tx, network, consensus_constants.num_preshards, genesis_council)?;
            tx.db()
                .cf(DatabaseMigrationVersion)?
                .put(&ByteColumn, &CURRENT_VERSION, OPERATION)?;
        },
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use tari_ootle_storage::{StateStore, StorageError};
    use tari_state_store_rocksdb::{DatabaseOptions, RocksDbStateStore};
    use tempfile::TempDir;

    use super::*;

    const OPERATION: &str = "test";

    fn store_stamped(version: u64) -> (RocksDbStateStore<String>, TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let db = RocksDbStateStore::<String>::open(tmp.path().join("rocksdb"), DatabaseOptions::default()).unwrap();
        db.with_write_tx(|tx| {
            tx.db()
                .cf(DatabaseMigrationVersion)?
                .put(&ByteColumn, &version, OPERATION)?;
            Ok::<_, StorageError>(())
        })
        .unwrap();
        (db, tmp)
    }

    fn run_migrate(db: &RocksDbStateStore<String>) -> anyhow::Result<()> {
        db.with_write_tx(|tx| {
            migrate(
                tx,
                Network::LocalNet,
                &ConsensusConstants::devnet(1),
                &GenesisCouncil::none(),
            )
        })
    }

    fn stored_version(db: &RocksDbStateStore<String>) -> u64 {
        let tx = db.create_read_tx().unwrap();
        tx.db()
            .cf(DatabaseMigrationVersion)
            .unwrap()
            .get(&ByteColumn, OPERATION)
            .unwrap()
    }

    #[test]
    fn a_database_at_the_current_version_is_accepted() {
        let (db, _tmp) = store_stamped(CURRENT_VERSION);
        run_migrate(&db).unwrap();
        assert_eq!(stored_version(&db), CURRENT_VERSION);
    }

    #[test]
    fn a_database_newer_than_the_binary_is_refused() {
        let (db, _tmp) = store_stamped(CURRENT_VERSION + 1);
        let err = run_migrate(&db).unwrap_err();
        assert!(err.to_string().contains("is newer than this binary"), "{err}");
        assert_eq!(stored_version(&db), CURRENT_VERSION + 1);
    }
}
