//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_ootle_common_types::{Epoch, NodeAddressable, optional::Optional};
use tari_ootle_storage::global::{BlockHeaderModel, GlobalDb};
use tari_ootle_storage_sqlite::global::SqliteGlobalDbAdapter;
use tari_sidechain::FixedHash;
use tari_template_lib_types::Hash32;

pub trait BaseLayerBlockHeaderStore {
    fn add_block_headers<I: IntoIterator<Item = BlockHeaderModel>>(&self, headers: I) -> anyhow::Result<()>;

    /// Returns the stored header with the given block hash, or `None` if it is not stored. Used during
    /// reorg recovery to find the highest base-layer height whose canonical block we have already stored.
    /// `max_epoch` bounds the lookup to headers stored at or below that epoch.
    fn find_block_header_by_hash(
        &self,
        max_epoch: Epoch,
        block_hash: &FixedHash,
    ) -> anyhow::Result<Option<BlockHeaderModel>>;

    /// Returns the lowest-height stored header in the given epoch, or `None` if none are stored. Used
    /// during reorg recovery to recover the epoch's boundary block hash.
    fn get_first_block_header_in_epoch(&self, epoch: Epoch) -> anyhow::Result<Option<BlockHeaderModel>>;

    /// Deletes every stored header with a height strictly greater than `height`, returning the number of
    /// rows removed. Used to discard headers orphaned by a base-layer reorg.
    fn delete_block_headers_above(&self, height: u64) -> anyhow::Result<usize>;
}

impl<TAddr: NodeAddressable> BaseLayerBlockHeaderStore for GlobalDb<SqliteGlobalDbAdapter<TAddr>> {
    fn add_block_headers<I: IntoIterator<Item = BlockHeaderModel>>(&self, headers: I) -> anyhow::Result<()> {
        let mut tx = self.create_transaction()?;
        let mut header_db = self.block_headers(&mut tx);
        for header in headers {
            header_db.insert(header)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn find_block_header_by_hash(
        &self,
        max_epoch: Epoch,
        block_hash: &FixedHash,
    ) -> anyhow::Result<Option<BlockHeaderModel>> {
        let block_hash = Hash32::from_array(block_hash.into_array());
        let mut tx = self.create_transaction()?;
        let header = self
            .block_headers(&mut tx)
            .get_by_hash(max_epoch, &block_hash)
            .optional()?;
        Ok(header)
    }

    fn get_first_block_header_in_epoch(&self, epoch: Epoch) -> anyhow::Result<Option<BlockHeaderModel>> {
        let mut tx = self.create_transaction()?;
        let header = self
            .block_headers(&mut tx)
            .get_first_block_header_in_epoch(epoch)
            .optional()?;
        Ok(header)
    }

    fn delete_block_headers_above(&self, height: u64) -> anyhow::Result<usize> {
        let mut tx = self.create_transaction()?;
        let num_deleted = self.block_headers(&mut tx).delete_above(height)?;
        tx.commit()?;
        Ok(num_deleted)
    }
}
