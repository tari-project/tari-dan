//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    any::type_name,
    borrow::Borrow,
    fmt::{Debug, Display},
    ops::Range,
};

use rocksdb::{ColumnFamily, IterateBounds, IteratorMode, TransactionDB};
use tari_ootle_common_types::displayable::Displayable;
use tari_ootle_storage::Ordering;

use crate::{
    codecs::{DbCodec, DbDecoder, DbEncoder, EncodeVec, PrefixCodec, UnitCodec},
    error::RocksDbStorageError,
    traits::{Cf, PrefixedCodec, QueryCf, RocksReader, RocksWriter},
};

pub struct DbContext<'db, TX> {
    db: &'db TransactionDB,
    tx: &'db TX,
}

impl<'db, TX> DbContext<'db, TX> {
    pub(crate) fn new(db: &'db TransactionDB, tx: &'db TX) -> Self {
        Self { db, tx }
    }

    pub fn cf<CF: Cf>(&self, _cf: CF) -> Result<CfContext<'db, TX, CF>, RocksDbStorageError> {
        let handle = self
            .db
            .cf_handle(CF::name())
            .ok_or_else(|| RocksDbStorageError::ColumnFamilyNotFound {
                operation: "create CF context",
                cf: format!("CF={}, cf_name={}", type_name::<CF>(), CF::name()),
            })?;
        CfContext::create(self.tx, handle)
    }
}

pub struct CfContext<'db, DB, CF: Cf> {
    db: &'db DB,
    handle: &'db ColumnFamily,
    key_codec: PrefixCodec<CF::Prefix, CF::KeyCodec>,
    value_codec: CF::ValueCodec,
}

impl<'db, DB, CF: Cf> CfContext<'db, DB, CF> {
    pub(crate) fn create(db: &'db DB, handle: &'db ColumnFamily) -> Result<Self, RocksDbStorageError> {
        let key_codec = CF::key_codec();
        let value_codec = CF::value_codec();
        Ok(Self {
            db,
            handle,
            key_codec,
            value_codec,
        })
    }
}

impl<'db, CF: Cf, DB: RocksReader> CfContext<'db, DB, CF> {
    pub fn encode_key(&self, key: &CF::Key) -> EncodeVec {
        self.key_codec.encode(key).unwrap_or_else(|e| {
            panic!(
                "database corruption: key encoding failed for CF[{}], key type {}: {}",
                CF::name(),
                type_name::<CF::Key>(),
                e
            )
        })
    }

    pub fn get_raw_key(&self, key: &[u8], operation: &'static str) -> Result<CF::Value, RocksDbStorageError> {
        let value = self
            .db
            .get_pinned_cf(self.handle, key)
            .map_err(|e| RocksDbStorageError::RocksDbError { operation, source: e })?;
        let bytes = value.ok_or_else(|| RocksDbStorageError::NotFound {
            key: Box::new(EncodeVec::from_slice(key)),
            operation,
        })?;
        let value = self.value_codec.decode_exact(&bytes)?;
        Ok(value)
    }

    pub fn get(&self, key: &CF::Key, operation: &'static str) -> Result<CF::Value, RocksDbStorageError> {
        let key = self.encode_key(key);
        self.get_raw_key(key.as_slice(), operation)
    }

    pub fn get_raw_pinned(
        &self,
        key: &CF::Key,
        operation: &'static str,
    ) -> Result<Option<rocksdb::DBPinnableSlice<'_>>, RocksDbStorageError> {
        let key = self.encode_key(key);
        let value = self
            .db
            .get_pinned_cf(self.handle, &key)
            .map_err(|e| RocksDbStorageError::RocksDbError { operation, source: e })?;
        Ok(value)
    }

    /// Counts the number of entries in the column family.
    /// WARNING: This operation is O(n) and can be slow for large column families/prefixed tables.
    pub fn count(&self, operation: &'static str) -> Result<usize, RocksDbStorageError> {
        let key_prefix = CF::key_prefix().map(|b| [b]);
        let prefix_bytes = match key_prefix {
            Some(ref b) => b.as_slice(),
            None => &[],
        };
        let mut opts = rocksdb::ReadOptions::default();
        opts.set_iterate_range(rocksdb::PrefixRange(prefix_bytes));
        let iter = self.db.iterator_cf_opt(self.handle, opts, IteratorMode::Start, |x| {
            x.map_err(|e| RocksDbStorageError::RocksDbError { operation, source: e })
                .map(|_| ())
        });
        let mut count = 0;
        for result in iter {
            let () = result?;
            count += 1;
        }
        Ok(count)
    }

    /// Counts the number of entries with the given prefix in the column family.
    /// WARNING: This operation is O(n) and can be slow for large prefixed tables.
    pub fn count_prefix(&self, prefix: &CF::Key) -> Result<usize, RocksDbStorageError> {
        let prefix = self.encode_key(prefix);
        let mut opts = rocksdb::ReadOptions::default();
        opts.set_iterate_range(rocksdb::PrefixRange(prefix));
        let iter = self.db.iterator_cf_opt(self.handle, opts, IteratorMode::Start, |x| {
            x.map_err(|e| RocksDbStorageError::RocksDbError {
                operation: "count_prefix",
                source: e,
            })
            .map(|_| ())
        });
        let mut count = 0;
        for result in iter {
            let () = result?;
            count += 1;
        }
        Ok(count)
    }

    /// Returns true if the key exists in the column family, otherwise false.
    pub fn exists(&self, key: &CF::Key, operation: &'static str) -> Result<bool, RocksDbStorageError> {
        let key = self.encode_key(key);
        let value = self
            .db
            .get_pinned_cf(self.handle, key)
            .map_err(|e| RocksDbStorageError::RocksDbError { operation, source: e })?;
        Ok(value.is_some())
    }

    pub fn exists_prefix(&self, prefix: &CF::Key) -> Result<bool, RocksDbStorageError> {
        self.any_exists_within_range(rocksdb::PrefixRange(self.encode_key(prefix)))
    }

    /// Returns true if any key exists within the given range, otherwise false.
    /// WARNING: the caller must encode the prefix correctly in the range bounds according to the column family's key
    /// codec.
    pub fn any_exists_within_range(&self, range: impl IterateBounds) -> Result<bool, RocksDbStorageError> {
        let mut opts = rocksdb::ReadOptions::default();
        opts.set_iterate_range(range);
        let mut iter = self.db.iterator_cf_opt(self.handle, opts, IteratorMode::Start, |res| {
            res.map_err(|e| RocksDbStorageError::RocksDbError {
                operation: "any_exists_within_range",
                source: e,
            })
            .map(|_| ())
        });
        let next = iter.next().transpose()?;
        Ok(next.is_some())
    }

    /// Returns the value for the given keys if the exists. If the key does not exist, it is skipped.
    pub fn multi_get<I, T>(&self, keys: I, operation: &'static str) -> Result<Vec<CF::Value>, RocksDbStorageError>
    where
        I: IntoIterator<Item = T>,
        T: Borrow<CF::Key>,
    {
        let mut keys = keys.into_iter().peekable();
        if keys.peek().is_none() {
            return Ok(Vec::new());
        }

        let keys = keys.map(|k| {
            // We don't support key encoding failing here for mem allocation reasons. Generally, key encoding is
            // infallible so we should evaluate whether to change the codec to be infallible. If key encoding on the
            // database level ever fails a crash is reasonable.
            let key = self.key_codec.encode(k.borrow()).expect("Failed to encode key");
            (self.handle, key)
        });
        let results = self.db.multi_get_cf(keys);

        let mut values = Vec::with_capacity(results.len());

        for result in results {
            let maybe_value = result.map_err(|e| RocksDbStorageError::RocksDbError { operation, source: e })?;
            let Some(value) = maybe_value else {
                continue;
            };
            let value = self.value_codec.decode_exact(&value)?;
            values.push(value);
        }

        Ok(values)
    }

    /// Returns the value for each of the given keys, in the order given. Unlike [`Self::multi_get`], a key with no
    /// value is an error, so the result lines up one-to-one with the keys.
    pub fn multi_get_exact<I, T>(&self, keys: I, operation: &'static str) -> Result<Vec<CF::Value>, RocksDbStorageError>
    where
        I: IntoIterator<Item = T>,
        T: Borrow<CF::Key>,
    {
        let encoded = keys
            .into_iter()
            .map(|k| self.key_codec.encode(k.borrow()))
            .collect::<Result<Vec<_>, _>>()?;
        if encoded.is_empty() {
            return Ok(Vec::new());
        }

        let results = self
            .db
            .multi_get_cf(encoded.iter().map(|key| (self.handle, key.as_slice())));
        results
            .into_iter()
            .zip(&encoded)
            .map(|(result, key)| {
                let value = result
                    .map_err(|e| RocksDbStorageError::RocksDbError { operation, source: e })?
                    .ok_or_else(|| RocksDbStorageError::NotFound {
                        key: Box::new(key.clone()),
                        operation,
                    })?;
                self.value_codec.decode_exact(&value)
            })
            .collect()
    }

    pub fn iterator(
        &self,
        ordering: Ordering,
        operation: &'static str,
    ) -> impl Iterator<Item = Result<(CF::Key, CF::Value), RocksDbStorageError>> + '_ {
        let mode = ordering_to_mode(ordering);
        let key_prefix = CF::key_prefix().map(|b| [b]);
        let prefix_bytes = match key_prefix {
            Some(ref b) => b.as_slice(),
            None => &[],
        };
        let opts = create_prefixed_read_opts(prefix_bytes);
        self.db.iterator_cf_opt(self.handle, opts, mode, move |res| {
            res.map_err(|e| RocksDbStorageError::RocksDbError { operation, source: e })
                .and_then(|(k, v)| Ok((self.key_codec.decode_exact(k)?, self.value_codec.decode_exact(v)?)))
        })
    }

    pub fn key_iterator(
        &self,
        ordering: Ordering,
        operation: &'static str,
    ) -> impl Iterator<Item = Result<CF::Key, RocksDbStorageError>> + '_ {
        let mode = ordering_to_mode(ordering);
        let key_prefix = CF::key_prefix().map(|b| [b]);
        let prefix_bytes = match key_prefix {
            Some(ref b) => b.as_slice(),
            None => &[],
        };
        let opts = create_prefixed_read_opts(prefix_bytes);
        self.db.iterator_cf_opt(self.handle, opts, mode, move |res| {
            res.map_err(|e| RocksDbStorageError::RocksDbError { operation, source: e })
                .and_then(|(k, _)| self.key_codec.decode_exact(k))
        })
    }

    pub fn value_iterator(
        &self,
        ordering: Ordering,
        operation: &'static str,
    ) -> impl Iterator<Item = Result<CF::Value, RocksDbStorageError>> + '_ {
        let mode = ordering_to_mode(ordering);
        let key_prefix = CF::key_prefix().map(|b| [b]);
        let prefix_bytes = match key_prefix {
            Some(ref b) => b.as_slice(),
            None => &[],
        };
        let opts = create_prefixed_read_opts(prefix_bytes);
        self.db.iterator_cf_opt(self.handle, opts, mode, move |res| {
            res.map_err(|e| RocksDbStorageError::RocksDbError { operation, source: e })
                .and_then(|(_, v)| self.value_codec.decode_exact(v))
        })
    }

    /// Returns an iterator over the values whose keys are at or after `start` (when given) and before `end` (when
    /// given). An open bound extends to the edge of the table.
    pub fn table_range_value_iterator(
        &self,
        ordering: Ordering,
        start: Option<&CF::Key>,
        end: Option<&CF::Key>,
        operation: &'static str,
    ) -> impl Iterator<Item = Result<CF::Value, RocksDbStorageError>> + '_ {
        let mode = ordering_to_mode(ordering);
        let bounds = TableBounds::for_cf::<CF>(start.map(|k| self.encode_key(k)), end.map(|k| self.encode_key(k)));
        self.db
            .iterator_cf_opt(self.handle, range_opts(bounds), mode, move |res| {
                res.map_err(|e| RocksDbStorageError::RocksDbError { operation, source: e })
                    .and_then(|(_, v)| self.value_codec.decode_exact(v))
            })
    }

    pub fn range_iterator(
        &self,
        ordering: Ordering,
        range: impl IterateBounds,
    ) -> impl Iterator<Item = Result<(CF::Key, CF::Value), RocksDbStorageError>> + '_ {
        let mode = ordering_to_mode(ordering);
        let opts = range_opts(range);
        self.db.iterator_cf_opt(self.handle, opts, mode, move |res| {
            res.map_err(|e| RocksDbStorageError::RocksDbError {
                operation: "range_iterator_with_codecs",
                source: e,
            })
            .and_then(|(k, v)| Ok((self.key_codec.decode_exact(k)?, self.value_codec.decode_exact(v)?)))
        })
    }

    fn range_iterator_with_codecs<KC, K, VC, V>(
        &self,
        ordering: Ordering,
        range: impl IterateBounds,
    ) -> impl Iterator<Item = Result<(K, V), RocksDbStorageError>> + 'db
    where
        K: 'static,
        V: 'static,
        KC: DbCodec<K> + Default,
        VC: DbCodec<V> + Default,
    {
        let mode = ordering_to_mode(ordering);
        let opts = range_opts(range);
        self.db.iterator_cf_opt(self.handle, opts, mode, |res| {
            res.map_err(|e| RocksDbStorageError::RocksDbError {
                operation: "range_iterator_with_codecs",
                source: e,
            })
            .and_then(|(k, v)| Ok((KC::default().decode_exact(k)?, VC::default().decode_exact(v)?)))
        })
    }

    fn range_key_iterator_custom_codec<C, K>(
        &self,
        ordering: Ordering,
        range: impl IterateBounds,
    ) -> impl Iterator<Item = Result<K, RocksDbStorageError>> + 'db
    where
        K: 'static,
        C: DbCodec<K> + Default,
    {
        let iter = self.range_iterator_with_codecs::<C, K, UnitCodec, ()>(ordering, range);
        iter.map(|res| {
            let (k, _) = res?;
            Ok::<_, RocksDbStorageError>(k)
        })
    }

    pub fn prefix_range_iterator(
        &self,
        ordering: Ordering,
        key: &CF::Key,
    ) -> impl Iterator<Item = Result<(CF::Key, CF::Value), RocksDbStorageError>> + '_ {
        let key = self.encode_key(key);
        self.range_iterator(ordering, rocksdb::PrefixRange(key))
    }

    pub fn prefix_range_iterator_raw_key(
        &self,
        ordering: Ordering,
        key: impl Into<Vec<u8>>,
    ) -> impl Iterator<Item = Result<(CF::Key, CF::Value), RocksDbStorageError>> + '_ {
        self.range_iterator(ordering, rocksdb::PrefixRange(key))
    }

    pub fn prefix_range_key_iterator(
        &self,
        ordering: Ordering,
        key: &CF::Key,
    ) -> impl Iterator<Item = Result<CF::Key, RocksDbStorageError>> + '_ {
        let key = self.encode_key(key);
        let iter = self.range_iterator_with_codecs::<PrefixedCodec<CF>, CF::Key, UnitCodec, ()>(
            ordering,
            rocksdb::PrefixRange(key),
        );
        iter.map(|res| {
            let (k, _) = res?;
            Ok::<_, RocksDbStorageError>(k)
        })
    }

    pub fn prefix_range_value_iterator(
        &self,
        ordering: Ordering,
        key: &CF::Key,
    ) -> impl Iterator<Item = Result<CF::Value, RocksDbStorageError>> + '_ {
        let key = self.encode_key(key);
        let iter = self.range_iterator_with_codecs::<UnitCodec, (), CF::ValueCodec, CF::Value>(
            ordering,
            rocksdb::PrefixRange(key),
        );
        iter.map(|res| {
            let (_, v) = res?;
            Ok::<_, RocksDbStorageError>(v)
        })
    }
}

impl<CF, DB> CfContext<'_, DB, CF>
where
    CF: Cf,
    CF::Key: Default,
    DB: RocksReader,
{
    pub fn get_by_default_key(&self, operation: &'static str) -> Result<CF::Value, RocksDbStorageError> {
        self.get(&CF::Key::default(), operation)
    }
}

impl<CF: Cf, DB: RocksWriter> CfContext<'_, DB, CF> {
    /// Reads a value and holds the key's lock until the transaction ends. Use for a read whose result decides a
    /// write, so a concurrent writer of the same key waits until this transaction ends.
    pub fn get_for_update(&self, key: &CF::Key, operation: &'static str) -> Result<CF::Value, RocksDbStorageError> {
        let key = self.encode_key(key);
        let value = self
            .db
            .get_pinned_for_update_cf(self.handle, &key)
            .map_err(|e| RocksDbStorageError::RocksDbError { operation, source: e })?;
        let bytes = value.ok_or_else(|| RocksDbStorageError::NotFound {
            key: Box::new(key),
            operation,
        })?;
        self.value_codec.decode_exact(&bytes)
    }

    /// Like [`Self::exists`], and holds the key's lock until the transaction ends.
    pub fn exists_for_update(&self, key: &CF::Key, operation: &'static str) -> Result<bool, RocksDbStorageError> {
        let key = self.encode_key(key);
        let value = self
            .db
            .get_pinned_for_update_cf(self.handle, key)
            .map_err(|e| RocksDbStorageError::RocksDbError { operation, source: e })?;
        Ok(value.is_some())
    }

    pub fn insert(&self, key: &CF::Key, value: &CF::Value, operation: &'static str) -> Result<(), RocksDbStorageError> {
        if self.exists_for_update(key, operation)? {
            let key = self.encode_key(key);
            return Err(RocksDbStorageError::ConflictingInsert {
                key: Box::new(key),
                details: "Key already exists".to_string(),
            });
        }

        self.put(key, value, operation)
    }

    pub fn put(&self, key: &CF::Key, value: &CF::Value, operation: &'static str) -> Result<(), RocksDbStorageError> {
        let encoded_value = self.value_codec.encode(value)?;
        self.put_raw_value(key, &encoded_value, operation)
    }

    pub fn put_raw_value(
        &self,
        key: &CF::Key,
        encoded_value: &[u8],
        operation: &'static str,
    ) -> Result<(), RocksDbStorageError> {
        let key = self.key_codec.encode(key)?;

        self.db
            .put_cf(self.handle, &key, encoded_value)
            .map_err(|source| RocksDbStorageError::RocksDbError { operation, source })?;

        Ok(())
    }

    /// Checks if the key exists and deletes it. If the key does not exist, an error is returned.
    /// Prefer using `delete` in many cases to save a read operation. This method is useful when the implementer needs
    /// a NotFound error to be returned when the key does not exist.
    /// If the key is known to exist (i.e. just fetched via an iterator), should exist because it is in an index column
    /// family, or the implementer wants idempotency, use `delete`.
    pub fn delete_or_not_found(&self, key: &CF::Key, operation: &'static str) -> Result<(), RocksDbStorageError> {
        if !self.exists(key, operation)? {
            return Err(RocksDbStorageError::NotFound {
                key: Box::new(self.key_codec.encode(key)?),
                operation,
            });
        }
        self.delete(key, operation)?;

        Ok(())
    }

    pub fn delete(&self, key: &CF::Key, operation: &'static str) -> Result<(), RocksDbStorageError> {
        let key = self.key_codec.encode(key)?;
        self.db
            .delete_cf(self.handle, key)
            .map_err(|e| RocksDbStorageError::RocksDbError { operation, source: e })?;
        Ok(())
    }
}

pub type QueryCfKv<TQuery> = (
    <<TQuery as QueryCf>::Cf as Cf>::Key,
    <<TQuery as QueryCf>::Cf as Cf>::Value,
);

impl<'db, TQuery: QueryCf, DB: RocksReader> CfContext<'db, DB, TQuery> {
    pub fn query_prefix_range_key_iterator(
        &self,
        ordering: Ordering,
        key: &TQuery::Key,
    ) -> impl Iterator<Item = Result<<TQuery::Cf as Cf>::Key, RocksDbStorageError>> + 'db {
        let key = self.encode_key(key);
        self.range_key_iterator_custom_codec::<PrefixedCodec<<TQuery as QueryCf>::Cf>, <<TQuery as QueryCf>::Cf as Cf>::Key>(
            ordering,
            rocksdb::PrefixRange(key),
        )
    }

    pub fn query_prefix_range_iterator(
        &self,
        ordering: Ordering,
        prefix: &TQuery::Key,
    ) -> impl Iterator<Item = Result<QueryCfKv<TQuery>, RocksDbStorageError>> + 'db {
        let key = self.encode_key(prefix);
        self.range_iterator_with_codecs::<
            PrefixedCodec<<TQuery as QueryCf>::Cf>,
            <<TQuery as QueryCf>::Cf as Cf>::Key,
            <<TQuery as QueryCf>::Cf as Cf>::ValueCodec,
            <<TQuery as QueryCf>::Cf as Cf>::Value,
        >(ordering, rocksdb::PrefixRange(key))
    }

    /// Collects the keys of a prefix range before returning.
    ///
    /// A write transaction's iterator reads through the transaction's own write batch, and RocksDB documents that
    /// updating the batch at the key the iterator is standing on invalidates that position - the state only *may*
    /// recover once the iterator steps on (`rocksdb/utilities/write_batch_with_index.h`, `NewIteratorWithBase`). A loop
    /// that writes the rows it reads must therefore take them all before writing any.
    pub fn query_prefix_range_keys(
        &self,
        ordering: Ordering,
        key: &TQuery::Key,
    ) -> Result<Vec<<TQuery::Cf as Cf>::Key>, RocksDbStorageError> {
        self.query_prefix_range_key_iterator(ordering, key).collect()
    }

    /// Collects the entries of a prefix range before returning. See [`Self::query_prefix_range_keys`] for why.
    pub fn query_prefix_range_entries(
        &self,
        prefix: &TQuery::Key,
        ordering: Ordering,
    ) -> Result<Vec<QueryCfKv<TQuery>>, RocksDbStorageError> {
        self.query_prefix_range_iterator(ordering, prefix).collect()
    }

    /// Collects the keys of a range before returning. See [`Self::query_prefix_range_keys`] for why.
    pub fn query_range_keys<B: Borrow<TQuery::Key>>(
        &self,
        ordering: Ordering,
        range: Range<B>,
    ) -> Result<Vec<<TQuery::Cf as Cf>::Key>, RocksDbStorageError> {
        self.query_range_key_iterator(ordering, range).collect()
    }

    /// Collects up to the first `limit` keys of a range. See [`Self::query_prefix_range_keys`] for why.
    pub fn query_range_keys_limited<B: Borrow<TQuery::Key>>(
        &self,
        ordering: Ordering,
        range: Range<B>,
        limit: usize,
    ) -> Result<Vec<<TQuery::Cf as Cf>::Key>, RocksDbStorageError> {
        self.query_range_key_iterator(ordering, range).take(limit).collect()
    }

    /// Collects up to the first `limit` entries below `end_key`. See [`Self::query_prefix_range_keys`] for why.
    pub fn query_end_range_entries_limited(
        &self,
        ordering: Ordering,
        end_key: &TQuery::Key,
        limit: usize,
    ) -> Result<Vec<QueryCfKv<TQuery>>, RocksDbStorageError> {
        self.query_end_range_iterator(ordering, end_key).take(limit).collect()
    }

    /// Collects the entries below `end_key` before returning. See [`Self::query_prefix_range_keys`] for why.
    pub fn query_end_range_entries(
        &self,
        ordering: Ordering,
        end_key: &TQuery::Key,
    ) -> Result<Vec<QueryCfKv<TQuery>>, RocksDbStorageError> {
        self.query_end_range_iterator(ordering, end_key).collect()
    }

    pub fn query_prefix_range_value_iterator(
        &self,
        ordering: Ordering,
        prefix: &TQuery::Key,
    ) -> impl Iterator<Item = Result<<<TQuery as QueryCf>::Cf as Cf>::Value, RocksDbStorageError>> + 'db {
        let key = self.encode_key(prefix);
        let iter = self.range_iterator_with_codecs::<UnitCodec, (), <<TQuery as QueryCf>::Cf as Cf>::ValueCodec, <<TQuery as QueryCf>::Cf as Cf>::Value, >(ordering, rocksdb::PrefixRange(key));
        iter.map(|res| {
            let (_, v) = res?;
            Ok::<_, RocksDbStorageError>(v)
        })
    }

    /// Returns an iterator over the keys in the column family that are greater than or equal to the start key.
    pub fn query_start_range_key_iterator(
        &self,
        ordering: Ordering,
        start_key: &TQuery::Key,
    ) -> impl Iterator<Item = Result<<TQuery::Cf as Cf>::Key, RocksDbStorageError>> + 'db {
        let key = self.encode_key(start_key);
        let iter = self
            .range_iterator_with_codecs::<PrefixedCodec<TQuery::Cf>, <TQuery::Cf as Cf>::Key, UnitCodec, ()>(
                ordering,
                TableBounds::for_cf::<TQuery::Cf>(Some(key), None),
            );
        iter.map(|res| {
            let (k, _) = res?;
            Ok::<_, RocksDbStorageError>(k)
        })
    }

    /// Returns an iterator over the key/values in the column family that are greater than or equal to the start key.
    pub fn query_start_range_iterator(
        &self,
        ordering: Ordering,
        start_key: &TQuery::Key,
    ) -> impl Iterator<Item = Result<QueryCfKv<TQuery>, RocksDbStorageError>> + 'db {
        let key = self.encode_key(start_key);
        self
            .range_iterator_with_codecs::<
                PrefixedCodec<TQuery::Cf>,
                <TQuery::Cf as Cf>::Key,
                <TQuery::Cf as Cf>::ValueCodec,
                <TQuery::Cf as Cf>::Value
            >(ordering, TableBounds::for_cf::<TQuery::Cf>(Some(key), None))
    }

    /// Returns a decoded key value iterator over the range of keys (exclusive).
    pub fn query_range_iterator<B: Borrow<TQuery::Key>>(
        &self,
        ordering: Ordering,
        range: Range<B>,
    ) -> impl Iterator<Item = Result<QueryCfKv<TQuery>, RocksDbStorageError>> + 'db {
        let start = self.encode_key(range.start.borrow());
        let end = self.encode_key(range.end.borrow());
        self.range_iterator_with_codecs::<
            PrefixedCodec<TQuery::Cf>,
            <TQuery::Cf as Cf>::Key,
            <TQuery::Cf as Cf>::ValueCodec,
            <TQuery::Cf as Cf>::Value,
        >(ordering, start..end)
    }

    /// Returns an iterator over the range of keys (exclusive).
    pub fn query_range_key_iterator<B: Borrow<TQuery::Key>>(
        &self,
        ordering: Ordering,
        range: Range<B>,
    ) -> impl Iterator<Item = Result<<TQuery::Cf as Cf>::Key, RocksDbStorageError>> + 'db {
        let start = self.encode_key(range.start.borrow());
        let end = self.encode_key(range.end.borrow());
        let iter = self
            .range_iterator_with_codecs::<PrefixedCodec<TQuery::Cf>, <TQuery::Cf as Cf>::Key, UnitCodec, ()>(
                ordering,
                start..end,
            );
        iter.map(|res| {
            let (k, _) = res?;
            Ok::<_, RocksDbStorageError>(k)
        })
    }

    /// Returns an iterator over the keys in the column family that are less than (exclusive) to the end key.
    pub fn query_end_range_key_iterator(
        &self,
        ordering: Ordering,
        end_key: &TQuery::Key,
    ) -> impl Iterator<Item = Result<<TQuery::Cf as Cf>::Key, RocksDbStorageError>> + 'db {
        let key = self.encode_key(end_key);
        let iter = self
            .range_iterator_with_codecs::<PrefixedCodec<TQuery::Cf>, <TQuery::Cf as Cf>::Key, UnitCodec, ()>(
                ordering,
                TableBounds::for_cf::<TQuery::Cf>(None, Some(key)),
            );
        iter.map(|res| {
            let (k, _) = res?;
            Ok::<_, RocksDbStorageError>(k)
        })
    }

    /// Returns an iterator over the key/values in the column family that are less than (exclusive) to the end key.
    pub fn query_end_range_iterator(
        &self,
        ordering: Ordering,
        end_key: &TQuery::Key,
    ) -> impl Iterator<Item = Result<QueryCfKv<TQuery>, RocksDbStorageError>> + 'db {
        let key = self.encode_key(end_key);
        self.range_iterator_with_codecs::<
            PrefixedCodec<TQuery::Cf>,
            <TQuery::Cf as Cf>::Key,
            <TQuery::Cf as Cf>::ValueCodec,
            <TQuery::Cf as Cf>::Value,
        >(ordering, TableBounds::for_cf::<TQuery::Cf>(None, Some(key)))
    }

    /// Returns the last key/value in the logical table.
    pub fn query_last(&self, operation: &'static str) -> Result<QueryCfKv<TQuery>, RocksDbStorageError> {
        let mut iter = self.range_iterator_with_codecs::<
            PrefixedCodec<TQuery::Cf>,
            <TQuery::Cf as Cf>::Key,
            <TQuery::Cf as Cf>::ValueCodec,
            <TQuery::Cf as Cf>::Value,
        >(Ordering::Descending, TableBounds::for_cf::<TQuery::Cf>(None, None));

        iter.next().transpose()?.ok_or_else(|| RocksDbStorageError::QueryError {
            operation,
            details: format!("No values in TQuery {}", TQuery::name()),
        })
    }
}

impl<CF, DB: RocksReader> CfContext<'_, DB, CF>
where
    CF: Cf,
    CF::Key: Debug,
    CF::Value: Debug,
{
    #[allow(dead_code)]
    pub fn dump_debug(&self) {
        let iter = self.db.iterator_cf(self.handle, IteratorMode::Start);
        let mut count = 0;
        eprintln!("-------------- Dumping CF: {} ------------", CF::name());
        for result in iter {
            let (raw_key, value) = result.unwrap();
            let key = match self.key_codec.decode_exact(&raw_key) {
                Ok(key) => key,
                Err(e) => {
                    eprintln!("Error decoding key: {}: raw: {}", e, raw_key.display());
                    continue;
                },
            };
            let value = self.value_codec.decode_exact(&value).unwrap();
            eprintln!("Key: {:?}, raw: {}, Value: {:?}", key, hex::encode(&raw_key), value);
            count += 1;
        }
        eprintln!("Total: {}", count);
    }
}
impl<CF, DB> CfContext<'_, DB, CF>
where
    DB: RocksReader,
    CF: Cf,
    CF::Key: Display,
    CF::Value: Debug,
{
    #[allow(dead_code)]
    pub fn dump_display(&self) {
        let iter = self.db.iterator_cf(self.handle, IteratorMode::Start);
        let mut count = 0;
        eprintln!("-------------- Dumping CF: {} ------------", CF::name());
        for result in iter {
            let (key, value) = result.unwrap();
            let key = self.key_codec.decode_exact(&key).unwrap();
            let value = self.value_codec.decode_exact(&value).unwrap();
            eprintln!("Key: {}, Value: {:?}", key, value);
            count += 1;
        }
        eprintln!("Total: {}", count);
    }
}

fn ordering_to_mode(ordering: Ordering) -> IteratorMode<'static> {
    match ordering {
        Ordering::Ascending => IteratorMode::Start,
        Ordering::Descending => IteratorMode::End,
    }
}

fn create_prefixed_read_opts<P: Into<Vec<u8>>>(prefix: P) -> rocksdb::ReadOptions {
    let mut opts = rocksdb::ReadOptions::default();
    opts.set_iterate_range(rocksdb::PrefixRange(prefix));
    opts
}

/// Iterate bounds for a single logical table. Logical tables share a physical column family and are separated only by
/// the leading key prefix byte, so a bound the caller leaves open is closed at the extent of the table's prefix.
struct TableBounds {
    start: Option<Vec<u8>>,
    end: Option<Vec<u8>>,
}

impl TableBounds {
    fn for_cf<CF: Cf>(start: Option<EncodeVec>, end: Option<EncodeVec>) -> Self {
        let prefix = CF::key_prefix();
        Self {
            start: start.map(Into::into).or_else(|| prefix.map(|p| vec![p])),
            // A prefix of 0xFF is the last table in the column family, so it has no upper bound.
            end: end
                .map(Into::into)
                .or_else(|| prefix.and_then(|p| p.checked_add(1)).map(|p| vec![p])),
        }
    }
}

impl IterateBounds for TableBounds {
    fn into_bounds(self) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
        (self.start, self.end)
    }
}

fn range_opts(range: impl IterateBounds) -> rocksdb::ReadOptions {
    let mut opts = rocksdb::ReadOptions::default();
    opts.set_iterate_range(range);
    opts
}
