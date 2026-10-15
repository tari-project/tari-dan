//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::Deref;

use log::debug;
use tari_consensus_types::{HighPc, HighestSeenBlock, LastExecuted, LockedBlock, ProposalCertificate};
use tari_ootle_common_types::optional::Optional;
use tari_ootle_storage::{
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
    consensus_models::{Block, BookkeepingModel},
};

use crate::traits::CertificateStore;

const LOG_TARGET: &str = "tari::ootle::storage::block_store";

pub trait BlockStore {
    fn update_nodes<TTx, TFnOnLock, TFnOnCommit, E>(
        &self,
        tx: &mut TTx,
        on_lock_block: TFnOnLock,
        on_commit: TFnOnCommit,
    ) -> Result<HighPc, E>
    where
        TTx: StateStoreWriteTransaction + Deref,
        TTx::Target: StateStoreReadTransaction,
        TFnOnLock: FnMut(&mut TTx, &LockedBlock, &Block, &ProposalCertificate) -> Result<(), E>,
        TFnOnCommit: FnMut(&mut TTx, Block) -> Result<(), E>,
        E: From<StorageError>;
}

impl BlockStore for Block {
    fn update_nodes<TTx, TFnOnLock, TFnOnCommit, E>(
        &self,
        tx: &mut TTx,
        mut on_lock_block: TFnOnLock,
        mut on_commit: TFnOnCommit,
    ) -> Result<HighPc, E>
    where
        TTx: StateStoreWriteTransaction + Deref,
        TTx::Target: StateStoreReadTransaction,
        TFnOnLock: FnMut(&mut TTx, &LockedBlock, &Block, &ProposalCertificate) -> Result<(), E>,
        TFnOnCommit: FnMut(&mut TTx, Block) -> Result<(), E>,
        E: From<StorageError>,
    {
        if HighestSeenBlock::get(&**tx, self.epoch())
            .optional()?
            .is_none_or(|h| h.height < self.height())
        {
            self.as_highest_seen().set(tx)?;
        }
        let high_qc = self.justify().update_highest(tx)?;

        // b'' <- b*.justify.node i.e. the (possibly new) justified block
        let justified_node = Block::get(&**tx, &self.justify().calculate_block_id())?;

        // b' <- b''.justify.node
        let new_locked = Block::get(&**tx, &justified_node.justify().calculate_block_id())?;
        if new_locked.is_genesis() {
            return Ok(high_qc);
        }

        let current_locked = LockedBlock::get(&**tx, self.epoch())?;
        if new_locked.height() > current_locked.height {
            on_locked_block_recurse(
                tx,
                &current_locked,
                &new_locked,
                justified_node.justify(),
                &mut on_lock_block,
            )?;
            new_locked.as_locked().set(tx)?;
            // A parked block at or below the locked block can never be voted on
            let num_removed = tx.parked_blocks_remove_up_to(new_locked.epoch(), new_locked.height())?;
            if num_removed > 0 {
                debug!(
                    target: LOG_TARGET,
                    "🗑️ Removed {num_removed} parked block(s) at or below locked block {}",
                    new_locked,
                );
            }
        }

        // b <- b'.justify.node
        let commit_node = new_locked.justify().calculate_block_id();
        // b''.parent() == b'.id() && b'.parent() == b.id()
        if justified_node.parent() == new_locked.id() && *new_locked.parent() == commit_node {
            debug!(
                target: LOG_TARGET,
                "✅ Block {} {} forms a 3-chain b'' = {}, b' = {}, b = {}",
                self.height(),
                self.id(),
                justified_node.id(),
                new_locked.id(),
                commit_node,
            );

            // Commit prepare_node (b)
            if commit_node.is_zero() {
                return Ok(high_qc);
            }
            let prepare_node = Block::get(&**tx, &commit_node)?;
            let last_executed = LastExecuted::get(&**tx, self.epoch())?;
            let last_exec = prepare_node.as_last_executed();
            on_commit_block_recurse(tx, &last_executed, prepare_node, &mut on_commit)?;
            last_exec.set(tx)?;
        } else {
            debug!(
                target: LOG_TARGET,
                "Block {} {} DOES NOT form a 3-chain b'' = {}, b' = {}, b = {}, b* = {}",
                self.height(),
                self.id(),
                justified_node.id(),
                new_locked.id(),
                commit_node,
                self.id()
            );
        }

        Ok(high_qc)
    }
}

fn on_locked_block_recurse<TTx, F, E>(
    tx: &mut TTx,
    locked: &LockedBlock,
    block: &Block,
    justify_qc: &ProposalCertificate,
    callback: &mut F,
) -> Result<(), E>
where
    TTx: StateStoreWriteTransaction + Deref,
    TTx::Target: StateStoreReadTransaction,
    E: From<StorageError>,
    F: FnMut(&mut TTx, &LockedBlock, &Block, &ProposalCertificate) -> Result<(), E>,
{
    if locked.height < block.height() {
        let parent = block.get_parent(&**tx)?;
        on_locked_block_recurse(tx, locked, &parent, block.justify(), callback)?;
        callback(tx, locked, block, justify_qc)?;
    }
    Ok(())
}

fn on_commit_block_recurse<TTx, F, E>(
    tx: &mut TTx,
    last_executed: &LastExecuted,
    block: Block,
    callback: &mut F,
) -> Result<(), E>
where
    TTx: StateStoreWriteTransaction + Deref,
    TTx::Target: StateStoreReadTransaction,
    E: From<StorageError>,
    F: FnMut(&mut TTx, Block) -> Result<(), E>,
{
    if last_executed.height < block.height() {
        let parent = block.get_parent(&**tx)?;
        // Recurse to "catch up" any parent blocks we may not have executed
        on_commit_block_recurse(tx, last_executed, parent, callback)?;
        callback(tx, block)?;
    }
    Ok(())
}
