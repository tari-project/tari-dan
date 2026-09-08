//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::BTreeMap, io::Write, ops::Deref};

use rand::{Rng, RngExt};
use tari_bor::cbor;
use tari_common_types::types::FixedHash;
use tari_consensus_types::{BlockId, Decision, LeafBlock, PcId, ProposalCertificate, ShardGroupAccumulatedData};
use tari_engine_types::{
    component::{Component, ComponentBody, ComponentHeader},
    substate::{SubstateId, SubstateValue, hash_substate},
};
use tari_ootle_common_types::{
    Epoch,
    ExtraData,
    NodeHeight,
    NumPreshards,
    ProtocolVersion,
    ShardGroup,
    VersionedSubstateId,
    VersionedSubstateIdRef,
    hashing::tari_consensus_hasher,
    shard::Shard,
};
use tari_ootle_storage::{
    ShardScopedTreeStoreWriter,
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    consensus_models::{
        Block,
        BlockPledge,
        BookkeepingModel,
        Command,
        CommandsCommitProof,
        ForeignProposal,
        ForeignProposalRecord,
        SubstateCreated,
        SubstateRecord,
        SubstateUpdateBatch,
        SubstateValueOrHash,
        TransactionAtom,
    },
};
use tari_ootle_transaction::{Network, TransactionId};
use tari_sidechain::{CommitProofElement, QuorumDecision, SidechainBlockCommitProof, SidechainBlockHeader};
use tari_state_store_rocksdb::{DatabaseOptions, RocksDbStateStore};
use tari_state_tree::{SpreadPrefixStateTree, SubstateTreeChange, Version};
use tari_template_lib::types::{
    ComponentAddress,
    ComponentKey,
    EntityId,
    ObjectKey,
    SubstateOwnerRule,
    TemplateAddress,
    access_rules::ComponentAccessRules,
    crypto::SchnorrSignatureBytes,
};
use tari_utilities::epoch_time::EpochTime;
use tempfile::TempDir;

/// Every helper builds records for one network; the schema activation schedule is per network.
pub const NETWORK: Network = Network::LocalNet;

pub const fn num_preshards() -> NumPreshards {
    NumPreshards::P256
}

/// Create a RocksDbStateStore and a temporary directory
/// NOTE: this takes around 1.5 s on my machine (AMD Ryzen 9 5950X, SSD)
pub fn create_rocksdb() -> (RocksDbStateStore<String>, TempDir) {
    create_rocksdb_with_opts(DatabaseOptions::default())
}

pub fn create_rocksdb_with_opts(opts: DatabaseOptions) -> (RocksDbStateStore<String>, TempDir) {
    let temp_dir = tempfile::Builder::new().disable_cleanup(false).tempdir().unwrap();
    let db_file = temp_dir.path().join("rocksdb");
    (RocksDbStateStore::open(db_file, opts).unwrap(), temp_dir)
}

pub fn create_tx_atom() -> TransactionAtom {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    TransactionAtom {
        id: TransactionId::new(bytes),
        decision: Decision::Commit,
        evidence: Default::default(),
        transaction_fee: 0,
        leader_fee: None,
    }
}

pub fn create_random_substate_id() -> SubstateId {
    let entity_id = EntityId::from_array(random_fixed());
    let component_key = ComponentKey::new(random_fixed());
    SubstateId::Component(ComponentAddress::new(ObjectKey::new(entity_id, component_key)))
}

pub fn random_fixed<const SIZE: usize>() -> [u8; SIZE] {
    let mut bytes = [0u8; SIZE];
    rand::rng().fill_bytes(&mut bytes);
    bytes
}

pub fn random_bytes(size: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; size];
    rand::rng().fill_bytes(&mut bytes);
    bytes
}

pub fn transaction_id_from_seed(seed: u32) -> TransactionId {
    let mut buf = [0u8; TransactionId::byte_size()];
    let mut writer = &mut buf[..];
    let be_bytes = seed.to_be_bytes();
    (0..32 / 4).for_each(|_| writer.write_all(&be_bytes).unwrap());
    TransactionId::new(buf)
}

pub fn build_substate_record(substate_id: &SubstateId, version: u32, state_version: Version) -> SubstateRecord {
    let entity_id = substate_id.to_object_key().as_entity_id();
    let value = build_substate_value(Some(entity_id));
    let at_epoch = Epoch::zero();
    SubstateRecord {
        substate_id: substate_id.clone(),
        version,
        state_hash: hash_substate(NETWORK, &value, version, at_epoch),
        substate_value: Some(value),
        created: SubstateCreated {
            at_epoch,
            in_shard: VersionedSubstateIdRef::new(substate_id, version).to_shard(num_preshards()),
            at_state_version: state_version,
        },
        destroyed: None,
    }
}

pub fn build_substate_value(entity_id: Option<EntityId>) -> SubstateValue {
    let bytes = random_bytes(100);
    let entity_id = entity_id.unwrap_or_else(|| EntityId::from_array(random_fixed()));
    SubstateValue::Component(Component {
        header: ComponentHeader {
            template_address: TemplateAddress::default(),
            owner_rule: SubstateOwnerRule::None,
            access_rules: ComponentAccessRules::allow_all(),
            entity_id,
        },
        body: ComponentBody {
            state: cbor!({
                "foo" => "bar",
                "bytes" => tari_bor::Value::Bytes(bytes.to_vec()),
            }),
        },
    })
}

pub fn create_substate_update_batch<'a, I>(epoch: Epoch, changes: I) -> SubstateUpdateBatch
where I: IntoIterator<Item = &'a SubstateRecord> {
    let mut batch = SubstateUpdateBatch::new(NETWORK, epoch);
    for substate in changes {
        if let Some(destroyed) = &substate.destroyed {
            batch
                .with_transition(
                    substate.to_versioned_substate_id().to_shard(num_preshards()),
                    destroyed.at_state_version,
                )
                .push(tari_ootle_storage::consensus_models::SubstateTransition::Down {
                    id: VersionedSubstateId::new(substate.substate_id.clone(), substate.version),
                });
        } else {
            batch
                .with_transition(
                    substate.to_versioned_substate_id().to_shard(num_preshards()),
                    substate.created().at_state_version,
                )
                .push(tari_ootle_storage::consensus_models::SubstateTransition::Up {
                    id: substate.substate_id.clone(),
                    version: substate.version,
                    substate_or_hash: substate
                        .substate_value()
                        .map(|v| SubstateValueOrHash::Value(Box::new(v.clone())))
                        .unwrap_or_else(|| SubstateValueOrHash::Hash(*substate.state_hash())),
                });
        }
    }

    batch
}

pub fn substate_id_tx_seed(transaction_id: TransactionId, seed: u32) -> SubstateId {
    let mut buf = [0u8; EntityId::LENGTH];
    buf[..].copy_from_slice(&transaction_id.as_hash().as_slice()[..EntityId::LENGTH]);
    let entity_id = EntityId::from_array(buf);
    let mut buf = [0u8; ComponentKey::LENGTH];
    buf[..].copy_from_slice(&transaction_id.as_hash().as_slice()[..ComponentKey::LENGTH]);
    let len = buf.len();
    buf[len - size_of::<u32>()..].copy_from_slice(&seed.to_be_bytes());
    let component_key = ComponentKey::new(buf);
    SubstateId::Component(ComponentAddress::new(ObjectKey::new(entity_id, component_key)))
}

pub fn substate_id_seed(seed: u32) -> SubstateId {
    let mut buf = [0u8; EntityId::LENGTH];
    let end = size_of::<u32>().min(EntityId::LENGTH);
    buf[..end].copy_from_slice(&seed.to_be_bytes()[..end]);
    let entity_id = EntityId::from_array(buf);
    let mut buf = [0u8; ComponentKey::LENGTH];
    let end = size_of::<u32>().min(ComponentKey::LENGTH);
    buf[..end].copy_from_slice(&seed.to_be_bytes()[..end]);
    let component_key = ComponentKey::new(buf);
    SubstateId::Component(ComponentAddress::new(ObjectKey::new(entity_id, component_key)))
}

pub fn random_substate_id_for_shard(shard: Shard) -> SubstateId {
    let seed = shard.as_u32();
    let mut buf = [0u8; EntityId::LENGTH];
    let end = size_of::<u32>().min(EntityId::LENGTH);
    buf[..end].copy_from_slice(&seed.to_be_bytes()[..end]);
    let entity_id = EntityId::from_array(buf);
    let mut buf = [0u8; ComponentKey::LENGTH];
    rand::rng().fill_bytes(&mut buf);
    let component_key = ComponentKey::new(buf);
    SubstateId::Component(ComponentAddress::new(ObjectKey::new(entity_id, component_key)))
}

pub fn substate_value_for_entity(entity_id: EntityId) -> SubstateValue {
    SubstateValue::Component(Component {
        header: ComponentHeader {
            template_address: TemplateAddress::default(),
            owner_rule: SubstateOwnerRule::None,
            access_rules: ComponentAccessRules::allow_all(),
            entity_id,
        },
        body: ComponentBody {
            state: cbor!({
                "baz" => "bar",
                "bytes" => tari_bor::Value::Bytes(entity_id.as_bytes().to_vec()),
            }),
        },
    })
}

pub fn gen_substates(
    epoch: Epoch,
    state_version: Version,
    shard: Shard,
    n: usize,
    substate_version: u32,
) -> impl Iterator<Item = SubstateRecord> {
    (0..n).map(move |_| {
        let substate_id = random_substate_id_for_shard(shard);
        let value = substate_value_for_entity(substate_id.to_object_key().as_entity_id());
        SubstateRecord::new(NETWORK, substate_id, substate_version, value, SubstateCreated {
            at_epoch: epoch,
            in_shard: shard,
            at_state_version: state_version,
        })
    })
}

pub fn gen_substates_for_shards(
    epoch: Epoch,
    state_version: Version,
    shard_range: impl IntoIterator<Item = u32>,
    substate_version: u32,
) -> impl Iterator<Item = SubstateRecord> {
    shard_range.into_iter().map(move |i| {
        let substate_id = substate_id_seed(i);
        let value = substate_value_for_entity(substate_id.to_object_key().as_entity_id());
        let shard = VersionedSubstateIdRef::new(&substate_id, substate_version).to_shard(num_preshards());
        SubstateRecord::new(NETWORK, substate_id, substate_version, value, SubstateCreated {
            at_epoch: epoch,
            in_shard: shard,
            at_state_version: state_version,
        })
    })
}

// track_caller allows a panic to include the caller's location in the error message
#[track_caller]
pub fn assert_eq_debug<T>(a: &T, b: &T)
where T: std::fmt::Debug {
    assert_eq!(format!("{:?}", a), format!("{:?}", b));
}

pub fn create_random_block_id() -> BlockId {
    BlockId::new(create_random_hash())
}

pub fn create_random_hash() -> FixedHash {
    let rand_bytes: [u8; FixedHash::byte_size()] = rand::rng().random();
    FixedHash::new(rand_bytes)
}

pub fn create_block(parent: Option<&Block>) -> Block {
    let network = Network::LocalNet;

    let Some(parent) = parent else {
        return Block::zero_block(network, num_preshards());
    };

    let atom1 = create_tx_atom();

    // This prevents all blocks to have the same hash/id
    let random_merkle_root = create_random_hash();
    let shard_group = ShardGroup::all_shards(num_preshards());

    Block::create(
        network,
        ProtocolVersion::V0,
        *parent.id(),
        parent.justify().clone(),
        None,
        NodeHeight(1),
        Epoch(0),
        shard_group,
        Default::default(),
        // Need to have a command in, otherwise this block will not be included internally in the query because it
        // cannot cause a state change without any commands
        [Command::LocalPrepare(atom1.clone())].into_iter().collect(),
        random_merkle_root,
        Default::default(),
        SchnorrSignatureBytes::zero(),
        EpochTime::now().as_u64(),
        FixedHash::zero(),
        ShardGroupAccumulatedData::default(),
        ExtraData::default(),
    )
    .unwrap()
}

pub fn create_block_with_qc(parent: &LeafBlock) -> Block {
    let network = Network::LocalNet;

    let atom1 = create_tx_atom();

    // This prevents all blocks to have the same hash/id
    let random_merkle_root = create_random_hash();

    let qc = create_qc(parent);
    let shard_group = parent.shard_group();

    Block::create(
        network,
        ProtocolVersion::V0,
        *parent.block_id(),
        qc,
        None,
        parent.height() + NodeHeight(1),
        parent.epoch(),
        shard_group,
        Default::default(),
        // Need to have a command in, otherwise this block will not be included internally in the query because it
        // cannot cause a state change without any commands
        [Command::LocalPrepare(atom1.clone())].into_iter().collect(),
        random_merkle_root,
        Default::default(),
        SchnorrSignatureBytes::zero(),
        EpochTime::now().as_u64(),
        FixedHash::zero(),
        ShardGroupAccumulatedData::default(),
        ExtraData::default(),
    )
    .unwrap()
}
pub fn create_qc(block: &LeafBlock) -> ProposalCertificate {
    ProposalCertificate::new(
        *block.block_id().hash(),
        *block.block_id(),
        block.height(),
        block.epoch(),
        ShardGroup::all_shards(num_preshards()),
        vec![],
        QuorumDecision::Accept,
    )
}

pub fn create_chain(num_blocks: usize) -> Vec<Block> {
    let mut blocks = Vec::with_capacity(num_blocks);
    let block = create_block(None);
    let mut parent = block.as_leaf();
    blocks.push(block);
    for _ in 0..num_blocks {
        let block = create_block_with_qc(&parent);
        parent = block.as_leaf();
        blocks.push(block);
    }
    blocks
}

pub fn commit_chain<TTx>(tx: &mut TTx, chain: &[Block])
where
    TTx: StateStoreWriteTransaction + Deref,
    TTx::Target: StateStoreReadTransaction,
{
    for block in chain {
        block.insert(tx).unwrap();
        tx.proposal_certificates_save(block.justify()).unwrap();
    }
    let len = chain.len();
    if len < 3 {
        return;
    }

    chain[len - 3].as_locked().set(tx).unwrap();

    for block in &chain[..len - 3] {
        tx.blocks_set_qcs(block.id(), Some(&PcId::zero()), Some(&PcId::zero()))
            .unwrap();
    }

    chain.last().unwrap().as_leaf().set(tx).unwrap();
}

pub fn create_foreign_proposal(parent_id: BlockId, epoch: Epoch) -> ForeignProposalRecord {
    let shard_group = ShardGroup::all_shards(num_preshards());
    let qc1 = ProposalCertificate::new(
        *parent_id.hash(),
        parent_id,
        NodeHeight(1),
        epoch,
        shard_group,
        vec![],
        QuorumDecision::Accept,
    );

    let foreign_block = Block::create(
        Network::LocalNet,
        ProtocolVersion::V0,
        parent_id,
        qc1.clone(),
        None,
        NodeHeight(2),
        epoch,
        shard_group,
        Default::default(),
        Default::default(),
        Default::default(),
        1,
        SchnorrSignatureBytes::zero(),
        EpochTime::now().as_u64(),
        FixedHash::zero(),
        ShardGroupAccumulatedData::default(),
        ExtraData::new(),
    )
    .unwrap();
    let commit_proof = CommandsCommitProof::new_latest(vec![], SidechainBlockCommitProof {
        header: SidechainBlockHeader {
            network: foreign_block.network().as_byte(),
            protocol_version: foreign_block.header().protocol_version().as_u32(),
            parent_id: *parent_id.hash(),
            justify_id: *qc1.calculate_id().hash(),
            height: foreign_block.height().as_u64(),
            epoch: epoch.as_u64(),
            // Any hash will do
            epoch_hash: tari_consensus_hasher("dummy").chain(&epoch).finalize().into(),
            shard_group: tari_sidechain::ShardGroup {
                start: shard_group.start().as_u32(),
                end_inclusive: shard_group.end().as_u32(),
            },
            proposed_by: Default::default(),
            state_merkle_root: Default::default(),
            command_merkle_root: Default::default(),
            signature: Default::default(),
            accumulated_data: Default::default(),
            metadata_hash: Default::default(),
        },
        proof_elements: vec![CommitProofElement::QuorumCertificate(
            tari_sidechain::QuorumCertificate {
                header_hash: foreign_block.header().calculate_hash(),
                parent_id: *parent_id.hash(),
                epoch: foreign_block.epoch().as_u64(),
                height: foreign_block.height().as_u64(),
                protocol_version: foreign_block.header().protocol_version().as_u32(),
                signatures: vec![],
                decision: QuorumDecision::Accept,
            },
        )],
    });

    ForeignProposalRecord::new(ForeignProposal::new(commit_proof, BlockPledge::default()))
}

/// The state-tree version the substates committed by [`commit_substates`] land at.
pub const PROOF_TEST_TREE_VERSION: Version = 1;

/// Commits `substates` to the store and to their shards' state trees, as a validator does when a
/// block commits, so that proofs can be generated against the resulting shard-group root.
pub fn commit_substates(db: &impl StateStore, substates: &[SubstateRecord]) {
    let mut by_shard: BTreeMap<Shard, Vec<&SubstateRecord>> = BTreeMap::new();
    for substate in substates {
        by_shard.entry(substate.created().in_shard).or_default().push(substate);
    }

    let mut tx = db.create_write_tx().unwrap();
    Block::zero_block(NETWORK, num_preshards()).insert(&mut tx).unwrap();

    for (shard, substates) in &by_shard {
        let changes = substates.iter().map(|s| SubstateTreeChange::Up {
            id: s.to_versioned_substate_id(),
            value_hash: *s.state_hash(),
        });
        {
            let mut store = ShardScopedTreeStoreWriter::new(&mut tx, *shard);
            SpreadPrefixStateTree::new(&mut store)
                .batch_put_substate_changes(None, PROOF_TEST_TREE_VERSION, changes)
                .unwrap();
        }
        tx.state_tree_shard_versions_set(*shard, PROOF_TEST_TREE_VERSION)
            .unwrap();
    }

    tx.substates_commit_batch(create_substate_update_batch(Epoch::zero(), substates))
        .unwrap();
    tx.commit().unwrap();
}
