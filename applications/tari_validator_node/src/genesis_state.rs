//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, ops::Deref};

use serde::Serialize;
use tari_engine_types::{
    component::{Component, ComponentBody, ComponentHeader},
    resource_container::ResourceContainer,
    substate::{SubstateId, SubstateValue, hash_substate},
    vault::Vault,
};
use tari_ootle_app_utilities::{
    genesis_governance::GenesisCouncil,
    genesis_resources::{
        get_nft_faucet_resource,
        get_public_identity_resource,
        get_stealth_tari_resource,
        get_xtr_faucet_claim_resource,
    },
    shared_consts::txtr_faucet_initial_supply,
};
use tari_ootle_common_types::{
    Epoch,
    NodeAddressable,
    NumPreshards,
    SubstateVersion,
    VersionedSubstateId,
    VersionedSubstateIdRef,
    shard::Shard,
};
use tari_ootle_storage::{
    ShardScopedTreeStoreWriter,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
    consensus_models::{SubstateRecord, SubstateTransition, SubstateUpdateBatch},
};
use tari_ootle_transaction::Network;
use tari_state_tree::{SpreadPrefixStateTree, SubstateTreeChange, Version};
use tari_template_builtin::{NftFaucetState, XtrFaucetState};
use tari_template_lib::types::{
    EntityId,
    SubstateOwnerRule,
    access_rules::ComponentAccessRules,
    constants::{
        BURN_RATE_GOVERNANCE_COMPONENT_ADDRESS,
        NFT_FAUCET_COMPONENT_ADDRESS,
        PUBLIC_IDENTITY_RESOURCE_ADDRESS,
        STEALTH_TARI_RESOURCE_ADDRESS,
        XTR_FAUCET_COMPONENT_ADDRESS,
        XTR_FAUCET_VAULT_ADDRESS,
    },
    governance::BurnRateGovernanceState,
};

/// The state version (and epoch) that bootstrapped genesis state is committed at. Consensus and
/// state-sync state changes begin at version 1 (see the state-sync `start_state_version` logic).
const GENESIS_STATE_VERSION: Version = 0;

pub fn has_bootstrapped<TTx: StateStoreReadTransaction>(tx: &TTx) -> Result<bool, StorageError> {
    // Assume that if the public identity resource exists, then the rest of the state has been bootstrapped
    SubstateRecord::exists(
        tx,
        VersionedSubstateId::new(PUBLIC_IDENTITY_RESOURCE_ADDRESS, SubstateVersion::ZERO).as_versioned_ref(),
    )
}

pub fn create_genesis_state<TTx>(
    tx: &mut TTx,
    network: Network,
    num_preshards: NumPreshards,
    genesis_council: &GenesisCouncil,
) -> Result<(), StorageError>
where
    TTx: StateStoreWriteTransaction + Deref,
    TTx::Target: StateStoreReadTransaction,
    TTx::Addr: NodeAddressable + Serialize,
{
    if has_bootstrapped(&**tx)? {
        return Ok(());
    }

    let mut substates: Vec<(SubstateId, SubstateValue)> = Vec::new();

    let (public_identity_address, resource) = get_public_identity_resource();
    substates.push((public_identity_address.into(), resource.into()));

    let (xtr_address, xtr_resource) = get_stealth_tari_resource(network);
    substates.push((xtr_address.into(), xtr_resource.into()));

    substates.push(burn_rate_governance_substate(genesis_council));

    if network.is_testnet() {
        // Create tXTR faucet
        substates.extend(xtr_faucet_substates(network));
        // Create NFT faucet
        substates.extend(nft_faucet_substates());
    }

    commit_genesis_substates(tx, network, num_preshards, substates)?;

    Ok(())
}

/// The component the council moves the exhaust burn rate through.
///
/// Instantiated on every network, with an empty schedule and whatever council the node seats. The
/// address has to exist from genesis: it lives on the global shard, and state roots are taken over
/// that shard from the first block. A network that seats no council gets one owned by nobody, which
/// leaves the rate with the release-scheduled table.
///
/// The council is the owner rule and every method rule denies, so the engine admits a call only from
/// the owner. `ComponentAccessRules::new()` denies by default, which is the whole of the method
/// policy.
fn burn_rate_governance_substate(council: &GenesisCouncil) -> (SubstateId, SubstateValue) {
    let component = Component {
        header: ComponentHeader {
            template_address: tari_template_builtin::BURN_RATE_GOVERNANCE_TEMPLATE_ADDRESS,
            owner_rule: council.owner_rule(),
            access_rules: ComponentAccessRules::new(),
            entity_id: EntityId::default(),
        },
        body: ComponentBody::from_cbor_value(
            tari_bor::to_value(&BurnRateGovernanceState::new()).expect("BurnRateGovernanceState encode is infallible"),
        ),
    };

    (BURN_RATE_GOVERNANCE_COMPONENT_ADDRESS.into(), component.into())
}

fn xtr_faucet_substates(network: Network) -> Vec<(SubstateId, SubstateValue)> {
    let vault = Vault::new(ResourceContainer::Stealth {
        address: STEALTH_TARI_RESOURCE_ADDRESS,
        revealed_amount: txtr_faucet_initial_supply(network),
        locked_amount: Default::default(),
    });

    let component = Component {
        header: ComponentHeader {
            template_address: tari_template_builtin::XTR_FAUCET_TEMPLATE_ADDRESS,
            owner_rule: SubstateOwnerRule::None,
            access_rules: ComponentAccessRules::allow_all(),
            entity_id: EntityId::default(),
        },
        body: ComponentBody::from_cbor_value(
            tari_bor::to_value(&XtrFaucetState {
                vault: XTR_FAUCET_VAULT_ADDRESS,
            })
            .expect("XtrFaucetState encode is infallible"),
        ),
    };

    let (claim_resource_address, claim_resource) = get_xtr_faucet_claim_resource();

    vec![
        (XTR_FAUCET_VAULT_ADDRESS.into(), vault.into()),
        (XTR_FAUCET_COMPONENT_ADDRESS.into(), component.into()),
        (claim_resource_address.into(), claim_resource.into()),
    ]
}

fn nft_faucet_substates() -> Vec<(SubstateId, SubstateValue)> {
    let component = Component {
        header: ComponentHeader {
            template_address: tari_template_builtin::NFT_FAUCET_TEMPLATE_ADDRESS,
            owner_rule: SubstateOwnerRule::None,
            access_rules: ComponentAccessRules::allow_all(),
            entity_id: EntityId::default(),
        },
        body: ComponentBody::from_cbor_value(
            tari_bor::to_value(&NftFaucetState { serial_number: 0 }).expect("NftFaucetState encode is infallible"),
        ),
    };

    let (resource_address, resource) = get_nft_faucet_resource();

    vec![
        (NFT_FAUCET_COMPONENT_ADDRESS.into(), component.into()),
        (resource_address.into(), resource.into()),
    ]
}

/// Commits the genesis substates to both the substate store and the per-shard state tree (JMT) at
/// [`GENESIS_STATE_VERSION`].
///
/// Writing them to the state tree (not just the substate store) is what allows them to be
/// cryptographically proven on read. Without a tree leaf, immutable genesis substates such as the
/// TARI resource have no inclusion proof and verified reads fail with a leaf-key mismatch.
fn commit_genesis_substates<TTx>(
    tx: &mut TTx,
    network: Network,
    num_preshards: NumPreshards,
    substates: Vec<(SubstateId, SubstateValue)>,
) -> Result<(), StorageError>
where
    TTx: StateStoreWriteTransaction + Deref,
    TTx::Target: StateStoreReadTransaction,
{
    let mut batch = SubstateUpdateBatch::new(network, Epoch::zero());
    let mut tree_changes: HashMap<Shard, Vec<SubstateTreeChange>> = HashMap::new();

    for (substate_id, value) in substates {
        let shard = VersionedSubstateIdRef::new(&substate_id, SubstateVersion::ZERO).to_shard(num_preshards);
        let value_hash = hash_substate(network, &value, SubstateVersion::ZERO, Epoch::zero());

        batch
            .with_transition(shard, GENESIS_STATE_VERSION)
            .push(SubstateTransition::Up {
                id: substate_id.clone(),
                version: SubstateVersion::ZERO,
                substate_or_hash: value.into(),
            });

        tree_changes.entry(shard).or_default().push(SubstateTreeChange::Up {
            id: VersionedSubstateId::new(substate_id, SubstateVersion::ZERO),
            value_hash,
        });
    }

    SubstateRecord::commit_batch(tx, batch)?;

    // Commit the genesis leaves into each shard's state tree at version 0. Consensus then builds
    // version 1 onwards on top of this (next_version = current_version.unwrap_or(0) + 1).
    for (shard, changes) in tree_changes {
        let mut store = ShardScopedTreeStoreWriter::new(tx, shard);
        SpreadPrefixStateTree::new(&mut store)
            .batch_put_substate_changes(None, GENESIS_STATE_VERSION, changes)
            .map_err(|e| StorageError::QueryError {
                reason: format!("commit_genesis_substates: failed to write state tree for {shard}: {e}"),
            })?;
        store
            .set_state_version(GENESIS_STATE_VERSION)
            .map_err(|e| StorageError::QueryError {
                reason: format!("commit_genesis_substates: failed to set state version for {shard}: {e}"),
            })?;
    }

    Ok(())
}
