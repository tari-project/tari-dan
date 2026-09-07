//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use rocksdb::{SingleThreaded, Transaction, TransactionDB};
use tari_common_types::types::FixedHash;
use tari_consensus_types::BlockId;
use tari_ootle_common_types::{Epoch, NodeHeight};
use tari_ootle_storage::{Ordering, StorageError};
use tari_ootle_transaction::TransactionId;
use tempfile::TempDir;

use crate::{
    cf_api::DbContext,
    codecs::{BlockIdCodec, BytesCodec, EpochCodec, NumberCodec, Prefixed, UnitCodec},
    traits::{Cf, QueryCf},
};

pub fn create_rocksdb(cf_names: impl IntoIterator<Item = &'static str>) -> (TransactionDB<SingleThreaded>, TempDir) {
    let temp_dir = tempfile::Builder::new().disable_cleanup(false).tempdir().unwrap();
    let path = temp_dir.path().join("rocksdb");
    // Use the production options so that tests exercise the prefix extractor and memtable prefix bloom
    let (db_opts, _budget) = crate::store::build_default_store_opts(&crate::options::DatabaseOptions::default());
    let tx_opts = rocksdb::TransactionDBOptions::default();
    let cf_descriptors = cf_names
        .into_iter()
        .map(|name| rocksdb::ColumnFamilyDescriptor::new(name, db_opts.clone()));
    let db = TransactionDB::open_cf_descriptors(&db_opts, &tx_opts, path, cf_descriptors)
        .map_err(|e| StorageError::ConnectionError {
            reason: e.into_string(),
        })
        .unwrap();

    (db, temp_dir)
}

pub fn ctx<'a>(
    db: &'a TransactionDB<SingleThreaded>,
    tx: &'a Transaction<TransactionDB>,
) -> DbContext<'a, Transaction<'a, TransactionDB<SingleThreaded>>> {
    DbContext::new(db, tx)
}

fn block_id_from_seed(seed: u64) -> BlockId {
    let mut bytes = [0u8; 32];
    bytes[0..8].copy_from_slice(&seed.to_be_bytes());
    BlockId::new(FixedHash::new(bytes))
}

fn transaction_id_from_seed(seed: u64) -> TransactionId {
    let mut bytes = [0u8; 32];
    bytes[0..8].copy_from_slice(&seed.to_be_bytes());
    TransactionId::new(bytes)
}

pub struct TwoBlocksCf;

impl Cf for TwoBlocksCf {
    type Key = (BlockId, TransactionId);
    type KeyCodec = (BlockIdCodec, BytesCodec);
    type Prefix = ();
    type Value = u64;
    type ValueCodec = NumberCodec<Self::Value>;

    fn name() -> &'static str {
        "two_blocks"
    }
}

pub struct TwoBlocksPrefixQuery;

impl QueryCf for TwoBlocksPrefixQuery {
    type Cf = TwoBlocksCf;
    type Key = BlockId;
    type KeyCodec = BlockIdCodec;
}

pub struct EpochHeightBlock;

impl Cf for EpochHeightBlock {
    type Key = (Epoch, NodeHeight, BlockId);
    type KeyCodec = (EpochCodec, NumberCodec<NodeHeight>, BlockIdCodec);
    type Prefix = ();
    type Value = ();
    type ValueCodec = UnitCodec;

    fn name() -> &'static str {
        "epoch_height_block"
    }
}

pub struct ByEpochQuery;

impl QueryCf for ByEpochQuery {
    type Cf = EpochHeightBlock;
    type Key = Epoch;
    type KeyCodec = EpochCodec;
}

/// Two logical tables that share one physical column family, separated only by their key prefix byte.
const SHARED_CF: &str = "shared_prefixed";

pub struct LowerPrefix;

impl Prefixed for LowerPrefix {
    fn prefix() -> Option<u8> {
        Some(1)
    }
}

pub struct UpperPrefix;

impl Prefixed for UpperPrefix {
    fn prefix() -> Option<u8> {
        Some(2)
    }
}

pub struct LowerEpochBlockCf;

impl Cf for LowerEpochBlockCf {
    type Key = (Epoch, BlockId);
    type KeyCodec = (EpochCodec, BlockIdCodec);
    type Prefix = LowerPrefix;
    type Value = ();
    type ValueCodec = UnitCodec;

    fn name() -> &'static str {
        SHARED_CF
    }
}

pub struct LowerByEpochQuery;

impl QueryCf for LowerByEpochQuery {
    type Cf = LowerEpochBlockCf;
    type Key = Epoch;
    type KeyCodec = EpochCodec;
}

pub struct UpperEpochBlockCf;

impl Cf for UpperEpochBlockCf {
    type Key = (Epoch, BlockId);
    type KeyCodec = (EpochCodec, BlockIdCodec);
    type Prefix = UpperPrefix;
    type Value = ();
    type ValueCodec = UnitCodec;

    fn name() -> &'static str {
        SHARED_CF
    }
}

pub struct UpperByEpochQuery;

impl QueryCf for UpperByEpochQuery {
    type Cf = UpperEpochBlockCf;
    type Key = Epoch;
    type KeyCodec = EpochCodec;
}

#[test]
fn it_puts_and_retrieves() {
    const OP: &str = "it_puts_and_retrieves_in_single_transaction";
    let (db, _tmp) = create_rocksdb([TwoBlocksCf::name()]);
    let tx = db.transaction();
    let ctx = ctx(&db, &tx);
    let cf = ctx.cf(TwoBlocksCf).unwrap();
    let key = (block_id_from_seed(1), transaction_id_from_seed(2));
    cf.put(&key, &123, OP).unwrap();
    let v = cf.get(&key, OP).unwrap();
    assert_eq!(v, 123);
    assert!(cf.exists(&key, OP).unwrap());
}

#[test]
fn it_iterates_using_the_prefix() {
    const OP: &str = "it_iterates_using_the_prefix";
    let (db, _tmp) = create_rocksdb([TwoBlocksCf::name()]);
    let tx = db.transaction();
    let ctx = ctx(&db, &tx);
    let cf = ctx.cf(TwoBlocksCf).unwrap();
    let key = (block_id_from_seed(1), transaction_id_from_seed(2));
    cf.put(&key, &10, OP).unwrap();
    let key = (block_id_from_seed(1), transaction_id_from_seed(3));
    cf.put(&key, &11, OP).unwrap();
    let key = (block_id_from_seed(1), transaction_id_from_seed(4));
    cf.put(&key, &12, OP).unwrap();
    let key = (block_id_from_seed(2), transaction_id_from_seed(3));
    cf.put(&key, &2, OP).unwrap();
    let key = (block_id_from_seed(3), transaction_id_from_seed(4));
    cf.put(&key, &3, OP).unwrap();

    let query = ctx.cf(TwoBlocksPrefixQuery).unwrap();

    let iter = query.query_prefix_range_iterator(Ordering::default(), &block_id_from_seed(1));

    let mut count = 0;

    for (i, result) in iter.enumerate() {
        let ((b1, b2), v) = result.unwrap();
        assert_eq!(b1, block_id_from_seed(1));
        assert_eq!(b2, transaction_id_from_seed(2 + i as u64));
        assert_eq!(v, 10 + i as u64);
        count += 1;
    }

    assert_eq!(count, 3);
}

#[test]
fn it_iterates_over_epoch_heights_ordering() {
    const OP: &str = "it_iterates_over_epoch_heights_ordering";
    let (db, _tmp) = create_rocksdb([EpochHeightBlock::name()]);
    let tx = db.transaction();
    let ctx = ctx(&db, &tx);
    let cf = ctx.cf(EpochHeightBlock).unwrap();
    for key in (0..100).map(|i| (Epoch(0), NodeHeight(i), block_id_from_seed(i))) {
        cf.put(&key, &(), OP).unwrap();
    }
    for key in (0..100).map(|i| (Epoch(1), NodeHeight(i), block_id_from_seed(i))) {
        cf.put(&key, &(), OP).unwrap();
    }

    let query = ctx.cf(ByEpochQuery).unwrap();

    let iter = query.query_prefix_range_key_iterator(Ordering::Ascending, &Epoch(0));
    for (i, result) in iter.enumerate() {
        let (epoch, height, _) = result.unwrap();
        assert_eq!(epoch, Epoch(0));
        assert_eq!(height, NodeHeight(i as u64));
    }

    let iter = query.query_prefix_range_key_iterator(Ordering::Descending, &Epoch(0));
    for (i, result) in iter.enumerate() {
        let i = 100 - 1 - i as u64;
        let (epoch, height, _) = result.unwrap();
        assert_eq!(epoch, Epoch(0));
        assert_eq!(height, NodeHeight(i));
    }
}

fn seed_shared_prefixed_cf(ctx: &DbContext<'_, Transaction<'_, TransactionDB<SingleThreaded>>>) {
    const OP: &str = "seed_shared_prefixed_cf";
    let lower = ctx.cf(LowerEpochBlockCf).unwrap();
    let upper = ctx.cf(UpperEpochBlockCf).unwrap();
    for i in 0..5u64 {
        lower.put(&(Epoch(i), block_id_from_seed(i)), &(), OP).unwrap();
        upper.put(&(Epoch(i), block_id_from_seed(i)), &(), OP).unwrap();
    }
}

#[test]
fn end_range_iterator_does_not_read_the_preceding_prefix() {
    let (db, _tmp) = create_rocksdb([SHARED_CF]);
    let tx = db.transaction();
    let ctx = ctx(&db, &tx);
    seed_shared_prefixed_cf(&ctx);

    let query = ctx.cf(UpperByEpochQuery).unwrap();

    let epochs = query
        .query_end_range_iterator(Ordering::Ascending, &Epoch(3))
        .map(|res| res.map(|((epoch, _), ())| epoch))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(epochs, vec![Epoch(0), Epoch(1), Epoch(2)]);

    let epochs = query
        .query_end_range_key_iterator(Ordering::Descending, &Epoch(3))
        .map(|res| res.map(|(epoch, _)| epoch))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(epochs, vec![Epoch(2), Epoch(1), Epoch(0)]);
}

#[test]
fn start_range_iterator_does_not_read_the_following_prefix() {
    let (db, _tmp) = create_rocksdb([SHARED_CF]);
    let tx = db.transaction();
    let ctx = ctx(&db, &tx);
    seed_shared_prefixed_cf(&ctx);

    let query = ctx.cf(LowerByEpochQuery).unwrap();

    let epochs = query
        .query_start_range_iterator(Ordering::Ascending, &Epoch(3))
        .map(|res| res.map(|((epoch, _), ())| epoch))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(epochs, vec![Epoch(3), Epoch(4)]);

    let epochs = query
        .query_start_range_key_iterator(Ordering::Descending, &Epoch(3))
        .map(|res| res.map(|(epoch, _)| epoch))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(epochs, vec![Epoch(4), Epoch(3)]);
}

#[test]
fn query_last_does_not_read_the_following_prefix() {
    const OP: &str = "query_last_does_not_read_the_following_prefix";
    let (db, _tmp) = create_rocksdb([SHARED_CF]);
    let tx = db.transaction();
    let ctx = ctx(&db, &tx);
    seed_shared_prefixed_cf(&ctx);

    let ((epoch, block_id), ()) = ctx.cf(LowerByEpochQuery).unwrap().query_last(OP).unwrap();
    assert_eq!(epoch, Epoch(4));
    assert_eq!(block_id, block_id_from_seed(4));
}
