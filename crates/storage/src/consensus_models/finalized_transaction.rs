//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::FixedHash;
use tari_consensus_types::Decision;
use tari_ootle_common_types::hashing::finalized_transaction_hasher;
use tari_ootle_transaction::TransactionId;
use tari_state_tree::{KeyedProofTree, LeafKey, TreeHash};

use super::{BlockCommands, BlockError, Command};

/// A transaction's outcome as a leaf of the transaction merkle root of the block that finalizes it. The tree holds
/// one leaf per finalized transaction, at the transaction id, so a proof shows either the decision a block reached
/// for a transaction or that the block did not finalize it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalizedTransactionLeaf<'a> {
    pub transaction_id: &'a TransactionId,
    pub decision: Decision,
}

impl FinalizedTransactionLeaf<'_> {
    pub fn key(&self) -> LeafKey {
        transaction_leaf_key(self.transaction_id)
    }

    pub fn hash(&self) -> TreeHash {
        let hash: FixedHash = finalized_transaction_hasher()
            .chain(self.transaction_id)
            .chain(&self.decision)
            .finalize()
            .into();
        TreeHash::from(hash.into_array())
    }
}

/// The key a transaction's leaf sits at. Transaction ids are hashes, so they spread evenly over the key space.
pub fn transaction_leaf_key(transaction_id: &TransactionId) -> LeafKey {
    LeafKey::new(TreeHash::from(transaction_id.into_array()))
}

/// The tree over the transactions `commands` finalize: `LocalOnly`, `AllAccept` and `SomeAccept`.
pub fn build_finalized_transaction_tree(commands: &BlockCommands) -> Result<KeyedProofTree, BlockError> {
    let leaves = commands.iter().filter_map(Command::finalising).map(|atom| {
        let leaf = FinalizedTransactionLeaf {
            transaction_id: atom.id(),
            decision: atom.decision(),
        };
        (leaf.key(), leaf.hash())
    });
    Ok(KeyedProofTree::build(leaves)?)
}

#[cfg(test)]
mod tests {
    use tari_consensus_types::{BlockId, ProposalCertificate, ShardGroupAccumulatedData};
    use tari_engine_types::commit_result::AbortReason;
    use tari_ootle_common_types::{Epoch, ExtraData, NodeHeight, NumPreshards, ProtocolVersion, ShardGroup};
    use tari_ootle_transaction::Network;
    use tari_state_tree::TreeHash;
    use tari_template_lib_types::crypto::SchnorrSignatureBytes;

    use super::*;
    use crate::consensus_models::{Block, LocalOnlyAtom, MultiShardAtom};

    fn tx(seed: u8) -> TransactionId {
        TransactionId::new([seed; 32])
    }

    fn local_only(seed: u8, decision: Decision) -> Command {
        Command::LocalOnly(LocalOnlyAtom {
            id: tx(seed),
            decision,
            transaction_fee: 0,
            leader_fee: None,
        })
    }

    fn multi_shard(seed: u8, decision: Decision) -> MultiShardAtom {
        MultiShardAtom {
            id: tx(seed),
            decision,
            evidence: Default::default(),
            transaction_fee: 0,
            leader_fee: None,
        }
    }

    fn abort() -> Decision {
        Decision::Abort(AbortReason::ExecutionFailure)
    }

    /// Finalizes transactions 1 (LocalOnly abort), 2 (AllAccept commit) and 3 (SomeAccept abort). Transactions 4 and 5
    /// are only prepared or accepted locally.
    fn commands() -> BlockCommands {
        BlockCommands::init([
            local_only(1, abort()),
            Command::AllAccept(multi_shard(2, Decision::Commit)),
            Command::SomeAccept(multi_shard(3, abort())),
            Command::LocalPrepare(multi_shard(4, Decision::Commit)),
            Command::LocalAccept(multi_shard(5, Decision::Commit)),
        ])
        .unwrap()
    }

    fn block(protocol_version: ProtocolVersion, commands: BlockCommands) -> Result<Block, BlockError> {
        let shard_group = ShardGroup::all_shards(NumPreshards::P64);
        Block::create(
            Network::LocalNet,
            protocol_version,
            BlockId::zero(),
            ProposalCertificate::genesis(Epoch(1), shard_group),
            None,
            NodeHeight(2),
            Epoch(1),
            shard_group,
            Default::default(),
            commands,
            FixedHash::zero(),
            0,
            SchnorrSignatureBytes::zero(),
            1234,
            FixedHash::zero(),
            ShardGroupAccumulatedData::default(),
            ExtraData::new(),
        )
    }

    fn root_of(block: &Block) -> TreeHash {
        let root = block
            .header()
            .transaction_merkle_root()
            .expect("a version 2 block has a root");
        TreeHash::from(root.into_array())
    }

    #[test]
    fn a_block_before_version_2_has_no_transaction_merkle_root() {
        for protocol_version in [ProtocolVersion::V0, ProtocolVersion::V1] {
            let block = block(protocol_version, commands()).unwrap();
            assert_eq!(block.header().transaction_merkle_root(), None);
            assert!(matches!(
                block.compute_transaction_proof(&tx(1)),
                Err(BlockError::NoTransactionMerkleRoot { .. })
            ));
        }
    }

    #[test]
    fn each_finalized_transaction_proves_its_decision() {
        let block = block(ProtocolVersion::V2, commands()).unwrap();
        let root = root_of(&block);

        for (seed, decision) in [(1, abort()), (2, Decision::Commit), (3, abort())] {
            let (leaf, proof) = block.compute_transaction_proof(&tx(seed)).unwrap();
            let leaf = leaf.expect("the block finalizes this transaction");
            assert_eq!(leaf.decision, decision);
            proof.verify_inclusion(&root, &leaf.key(), &leaf.hash()).unwrap();

            let other = FinalizedTransactionLeaf {
                transaction_id: leaf.transaction_id,
                decision: if decision.is_commit() {
                    abort()
                } else {
                    Decision::Commit
                },
            };
            proof.verify_inclusion(&root, &other.key(), &other.hash()).unwrap_err();
        }
    }

    #[test]
    fn a_transaction_the_block_does_not_finalize_proves_absent() {
        let block = block(ProtocolVersion::V2, commands()).unwrap();
        let root = root_of(&block);

        for seed in [4, 5, 6] {
            let (leaf, proof) = block.compute_transaction_proof(&tx(seed)).unwrap();
            assert!(leaf.is_none());
            proof.verify_exclusion(&root, &transaction_leaf_key(&tx(seed))).unwrap();
        }
    }

    #[test]
    fn the_block_id_commits_to_each_decision() {
        let original = block(ProtocolVersion::V2, commands()).unwrap();

        let commands = BlockCommands::init(commands().into_iter().map(|cmd| {
            if cmd == local_only(1, abort()) {
                local_only(1, Decision::Abort(AbortReason::InsufficientFeesPaid))
            } else {
                cmd
            }
        }))
        .unwrap();
        let changed = block(ProtocolVersion::V2, commands).unwrap();

        assert_ne!(root_of(&original), root_of(&changed));
        assert_ne!(original.id(), changed.id());
    }

    #[test]
    fn an_empty_block_commits_to_the_empty_tree() {
        let block = block(ProtocolVersion::V2, BlockCommands::empty()).unwrap();
        assert_eq!(root_of(&block), tari_state_tree::SPARSE_MERKLE_PLACEHOLDER_HASH);
    }
}
