//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_ootle_common_types::{Epoch, optional::Optional};
use tari_ootle_light_client::{
    TrustedValidatorSet,
    TrustedValidatorSetHash,
    test_fixtures::{CheckpointSpec, NETWORK, ValidatorSetFixture, checkpoint_entry, walk_shard},
};
use tari_ootle_wallet_sdk::{
    models::ValidatorSetAnchor,
    storage::{CommittableStore, ReadableWalletStore, WalletStoreReader, WalletStoreWriter, WriteableWalletStore},
};
use tari_ootle_wallet_storage_sqlite::SqliteWalletStore;

/// The trusted sets of epochs 2 and 3: the first anchored on, the second handed over to.
fn walked_sets() -> (TrustedValidatorSetHash, TrustedValidatorSetHash) {
    let sets = (1..=3).map(ValidatorSetFixture::new).collect::<Vec<_>>();
    let anchored = TrustedValidatorSet::anchor_on_first_use(
        NETWORK,
        walk_shard(),
        &checkpoint_entry(CheckpointSpec::handover(&sets[0], &sets[1])),
    )
    .unwrap();
    let walked = anchored
        .advance(&checkpoint_entry(CheckpointSpec::handover(&sets[1], &sets[2])))
        .unwrap();
    (anchored.trusted_hash(), walked.trusted_hash())
}

fn store() -> SqliteWalletStore {
    let db = SqliteWalletStore::try_open(":memory:").unwrap();
    db.run_migrations().unwrap();
    db
}

#[test]
fn it_keeps_each_trusted_set_and_the_anchor_they_descend_from() {
    let (anchored, walked) = walked_sets();
    let db = store();
    let mut tx = db.create_write_tx().unwrap();
    assert!(tx.trusted_validator_sets_get_anchor().unwrap().is_none());
    tx.trusted_validator_sets_insert(&anchored, Some(ValidatorSetAnchor::FirstUse))
        .unwrap();
    tx.trusted_validator_sets_insert(&walked, None).unwrap();
    tx.commit().unwrap();
    drop(tx);

    let mut tx = db.create_read_tx().unwrap();
    assert_eq!(tx.trusted_validator_sets_get(Epoch(2)).unwrap(), anchored);
    assert_eq!(tx.trusted_validator_sets_get(Epoch(3)).unwrap(), walked);
    assert!(tx.trusted_validator_sets_get(Epoch(4)).optional().unwrap().is_none());
    let anchor = tx.trusted_validator_sets_get_anchor().unwrap().unwrap();
    assert_eq!(anchor.validator_set, anchored);
    assert_eq!(anchor.anchor, ValidatorSetAnchor::FirstUse);
}

#[test]
fn a_set_recorded_for_an_epoch_stays_as_it_is() {
    let (_, walked) = walked_sets();
    let db = store();
    let mut tx = db.create_write_tx().unwrap();
    tx.trusted_validator_sets_insert(&walked, None).unwrap();
    tx.trusted_validator_sets_insert(&walked, Some(ValidatorSetAnchor::Reanchor))
        .unwrap();
    tx.commit().unwrap();
    drop(tx);

    let mut tx = db.create_read_tx().unwrap();
    assert_eq!(tx.trusted_validator_sets_get(Epoch(3)).unwrap(), walked);
    assert!(tx.trusted_validator_sets_get_anchor().unwrap().is_none());
}

#[test]
fn clearing_forgets_every_trusted_set() {
    let (anchored, walked) = walked_sets();
    let db = store();
    let mut tx = db.create_write_tx().unwrap();
    tx.trusted_validator_sets_insert(&anchored, Some(ValidatorSetAnchor::FirstUse))
        .unwrap();
    tx.trusted_validator_sets_insert(&walked, None).unwrap();
    tx.trusted_validator_sets_clear().unwrap();
    tx.trusted_validator_sets_insert(&walked, Some(ValidatorSetAnchor::Reanchor))
        .unwrap();
    tx.commit().unwrap();
    drop(tx);

    let mut tx = db.create_read_tx().unwrap();
    assert!(tx.trusted_validator_sets_get(Epoch(2)).optional().unwrap().is_none());
    let anchor = tx.trusted_validator_sets_get_anchor().unwrap().unwrap();
    assert_eq!(anchor.validator_set, walked);
    assert_eq!(anchor.anchor, ValidatorSetAnchor::Reanchor);
}
