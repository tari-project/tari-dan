//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use log::*;
use tari_consensus_types::{BlockId, LeafBlock};
use tari_engine_types::{published_template::PublishedTemplate, substate::SubstateId};
use tari_ootle_common_types::{Epoch, NodeHeight, optional::Optional};
use tari_ootle_storage::{
    StateStoreReadTransaction,
    StorageError,
    consensus_models::{Block, SubstateChange},
};
use tari_template_lib_types::TemplateAddress;

use crate::{hotstuff::substate_store::PendingSubstateStore, traits::UncommittedChainTemplates};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::uncommitted_templates";

/// Templates published in blocks whose diffs this validator has saved but not yet committed to durable state.
///
/// Execution reads committed templates from durable state, which trails the chain a block builds on twice over: the
/// block's uncommitted ancestors are not in it, and neither are the ancestors a replica commits in the write
/// transaction it evaluates the block in. An entry is therefore removed only once its block's commit is durable, so
/// that durable state and this store together hold every template of the chain.
#[derive(Debug, Clone, Default)]
pub struct UncommittedTemplateStore {
    blocks: Arc<RwLock<HashMap<BlockId, BlockTemplates>>>,
}

#[derive(Debug)]
struct BlockTemplates {
    epoch: Epoch,
    height: NodeHeight,
    templates: Vec<(TemplateAddress, Arc<PublishedTemplate>)>,
}

impl UncommittedTemplateStore {
    /// Records the templates `changes`, the diff saved for `block`, publishes.
    pub fn record(&self, block: &LeafBlock, changes: &[SubstateChange]) {
        let templates = changes
            .iter()
            .filter_map(|change| {
                let SubstateId::Template(address) = change.substate_id() else {
                    return None;
                };
                let template = change.up_substate()?.substate_value().as_template()?;
                Some((address.as_template_address(), Arc::new(template.clone())))
            })
            .collect::<Vec<_>>();
        if templates.is_empty() {
            return;
        }
        debug!(
            target: LOG_TARGET,
            "Block {} publishes {} template(s)",
            block,
            templates.len()
        );
        self.blocks.write().unwrap().insert(block.block_id, BlockTemplates {
            epoch: block.epoch,
            height: block.height,
            templates,
        });
    }

    /// The templates published in the uncommitted ancestors of the block `store` evaluates. An entry's block is one of
    /// them if it is in the pending chain `store` builds on, or if it has been committed: a validator only evaluates a
    /// block that extends everything it has committed.
    pub fn templates_for<TTx: StateStoreReadTransaction>(
        &self,
        store: &PendingSubstateStore<TTx>,
    ) -> Result<UncommittedChainTemplates, StorageError> {
        let blocks = self.blocks.read().unwrap();
        if blocks.is_empty() {
            return Ok(UncommittedChainTemplates::default());
        }
        let mut templates = HashMap::new();
        for (block_id, block) in blocks.iter() {
            let is_ancestor = store.parent_chain().contains_pending(block_id) ||
                Block::get(store.read_transaction(), block_id)
                    .optional()?
                    .is_some_and(|b| b.is_committed());
            if is_ancestor {
                templates.extend(block.templates.iter().cloned());
            }
        }
        Ok(UncommittedChainTemplates::new(templates))
    }

    /// Removes the entries at or below the durably committed block at `epoch` and `height`. Those blocks are either
    /// committed, so their templates are in durable state, or orphaned, so no chain publishes them.
    pub fn prune_to(&self, epoch: Epoch, height: NodeHeight) {
        let mut blocks = self.blocks.write().unwrap();
        if blocks.is_empty() {
            return;
        }
        blocks.retain(|_, block| (block.epoch, block.height) > (epoch, height));
    }

    /// Records the templates published in every block descended from `committed` whose diff is saved.
    pub fn rebuild<TTx: StateStoreReadTransaction>(&self, tx: &TTx, committed: &BlockId) -> Result<(), StorageError> {
        let mut parents = vec![*committed];
        while let Some(parent) = parents.pop() {
            for child in tx.blocks_get_pending_ids_by_parent(&parent)? {
                let block = Block::get(tx, &child)?;
                if let Some(diff) = block.get_diff(tx).optional()? {
                    self.record(&block.as_leaf(), diff.changes());
                }
                parents.push(child);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tari_consensus_types::{ProposalCertificate, ShardGroupAccumulatedData};
    use tari_crypto::tari_utilities::epoch_time::EpochTime;
    use tari_engine_types::substate::Substate;
    use tari_ootle_common_types::{
        ExtraData,
        NumPreshards,
        ProtocolVersion,
        ShardGroup,
        SubstateVersion,
        shard::Shard,
    };
    use tari_ootle_storage::{StateStore, consensus_models::BlockDiff};
    use tari_ootle_transaction::Network;
    use tari_sidechain::{FixedHash, QuorumDecision};
    use tari_state_store_rocksdb::{DatabaseOptions, RocksDbStateStore};
    use tari_template_lib_types::crypto::SchnorrSignatureBytes;

    use super::*;

    fn child_of(parent: &Block, seed: u8) -> Block {
        let justify = ProposalCertificate::new(
            parent.header().calculate_hash(),
            *parent.parent(),
            parent.height(),
            parent.epoch(),
            ShardGroup::all_shards(NumPreshards::P256),
            vec![],
            QuorumDecision::Accept,
        );
        Block::create(
            Network::LocalNet,
            ProtocolVersion::V0,
            *parent.id(),
            justify,
            None,
            parent.height() + NodeHeight(1),
            parent.epoch(),
            ShardGroup::all_shards(NumPreshards::P256),
            Default::default(),
            Default::default(),
            FixedHash::new([seed; 32]),
            0,
            SchnorrSignatureBytes::zero(),
            EpochTime::now().as_u64(),
            FixedHash::zero(),
            ShardGroupAccumulatedData::default(),
            ExtraData::new(),
        )
        .unwrap()
    }

    fn leaf(height: u64) -> LeafBlock {
        LeafBlock {
            block_id: BlockId::from([u8::try_from(height).unwrap(); 32]),
            height: NodeHeight(height),
            epoch: Epoch(1),
            shard_group: ShardGroup::all_shards(NumPreshards::P256),
        }
    }

    fn publishes(n: u8) -> SubstateChange {
        let address = tari_engine_types::published_template::PublishedTemplateAddress::from_hash([n; 32].into());
        SubstateChange::Up {
            id: SubstateId::Template(address),
            shard: Shard::first(),
            substate: Box::new(Substate::new(SubstateVersion::ZERO, PublishedTemplate {
                template_name: "test".try_into().unwrap(),
                author: Default::default(),
                binary: vec![n].try_into().unwrap(),
                at_epoch: 0,
                metadata_hash: None,
            })),
        }
    }

    fn recorded(store: &UncommittedTemplateStore) -> Vec<NodeHeight> {
        let mut heights = store
            .blocks
            .read()
            .unwrap()
            .values()
            .map(|b| b.height)
            .collect::<Vec<_>>();
        heights.sort();
        heights
    }

    #[test]
    fn a_block_that_publishes_no_template_is_not_recorded() {
        let store = UncommittedTemplateStore::default();
        store.record(&leaf(1), &[]);
        assert!(recorded(&store).is_empty());
    }

    #[test]
    fn pruning_removes_blocks_at_or_below_the_committed_block() {
        let store = UncommittedTemplateStore::default();
        for height in 1..=4 {
            store.record(&leaf(height), &[publishes(u8::try_from(height).unwrap())]);
        }
        store.prune_to(Epoch(1), NodeHeight(2));
        assert_eq!(recorded(&store), vec![NodeHeight(3), NodeHeight(4)]);
    }

    #[test]
    fn rebuilding_records_every_saved_block_above_the_committed_block() {
        let committed = child_of(&Block::zero_block(Network::LocalNet, NumPreshards::P256), 1);
        let publishing = child_of(&committed, 2);
        let fork = child_of(&committed, 3);
        let without_diff = child_of(&publishing, 4);

        let dir = tempfile::tempdir().unwrap();
        let db = RocksDbStateStore::<String>::open(dir.path().join("db"), DatabaseOptions::default()).unwrap();
        db.with_write_tx(|tx| {
            for block in [&committed, &publishing, &fork, &without_diff] {
                block.insert(tx)?;
            }
            BlockDiff::insert(tx, publishing.id(), &[publishes(1)])?;
            BlockDiff::insert(tx, fork.id(), &[publishes(2)])?;
            Ok::<_, StorageError>(())
        })
        .unwrap();

        let store = UncommittedTemplateStore::default();
        db.with_read_tx(|tx| store.rebuild(tx, committed.id())).unwrap();
        assert_eq!(recorded(&store), vec![NodeHeight(2), NodeHeight(2)]);
    }
}
