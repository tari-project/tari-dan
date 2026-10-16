//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashMap, HashSet};

use log::*;
use tari_engine_types::{
    SubstateVersion,
    indexed_value::{IndexedValueError, IndexedWellKnownTypes},
    resource::Resource,
    substate::{Substate, SubstateId, SubstateValue},
};
use tari_indexer_client::types::SubstateProof;
use tari_ootle_common_types::{
    Epoch,
    SubstateRequirement,
    VersionedSubstateId,
    VersionedSubstateIdRef,
    displayable::Displayable,
    optional::{IsNotFoundError, Optional},
    substate_type::SubstateType,
};
use tari_ootle_light_client::LightClientError;
use tari_template_lib::types::{ResourceAddress, TemplateAddress, constants::TARI_TOKEN};

use crate::{
    apis::validator_set::{ValidatorSetApi, ValidatorSetApiError},
    models::SubstateModel,
    network::{FetchedSubstates, WalletNetworkInterface},
    sdk::IndexerTrust,
    storage::{WalletStorageError, WalletStore, WalletStoreReader, WalletStoreWriter},
};

const LOG_TARGET: &str = "tari::ootle::wallet_sdk::apis::substate";

/// How many epochs before the latest validator set the wallet trusts a substate proof may be anchored. The indexer
/// serves proofs anchored in the network's current epoch or the one before, and the wallet trusts the set of the
/// current epoch once the checkpoint ending the one before is served.
const MAX_PROOF_AGE_EPOCHS: u64 = 1;

pub struct SubstatesApi<'a, TStore, TNetworkInterface> {
    store: &'a TStore,
    network_interface: &'a TNetworkInterface,
    validator_set_api: ValidatorSetApi<'a, TStore, TNetworkInterface>,
    indexer_trust: IndexerTrust,
}

impl<'a, TStore, TNetworkInterface> SubstatesApi<'a, TStore, TNetworkInterface>
where
    TStore: WalletStore,
    TNetworkInterface: WalletNetworkInterface,
    TNetworkInterface::Error: IsNotFoundError,
{
    pub fn new(
        store: &'a TStore,
        network_interface: &'a TNetworkInterface,
        validator_set_api: ValidatorSetApi<'a, TStore, TNetworkInterface>,
        indexer_trust: IndexerTrust,
    ) -> Self {
        Self {
            store,
            network_interface,
            validator_set_api,
            indexer_trust,
        }
    }

    pub fn get_substate(&self, address: &SubstateId) -> Result<SubstateModel, SubstateApiError> {
        let substate = self.store.with_read_tx(|tx| tx.substates_get(address))?;
        Ok(substate)
    }

    pub fn list_substates(
        &self,
        filter_by_type: Option<SubstateType>,
        filter_by_template: Option<&TemplateAddress>,
        limit: Option<u64>,
        offset: Option<u64>,
    ) -> Result<Vec<SubstateModel>, SubstateApiError> {
        let mut tx = self.store.create_read_tx()?;
        let substates = tx.substates_get_all(filter_by_type, filter_by_template, limit, offset)?;
        Ok(substates)
    }

    pub async fn get_substate_from_network(&self, id: SubstateId) -> Result<Substate, SubstateApiError> {
        let mut map = self.get_substates_from_network(vec![id.clone()]).await?;
        map.remove(&id)
            .ok_or_else(|| SubstateApiError::SubstateDoesNotExist { address: id })
    }

    /// Fetches the latest version of each of `ids` that exists. Under [`IndexerTrust::Verify`] every substate returned
    /// is proven, as [`Self::fetch_substate_from_network`] describes, or the fetch fails.
    pub async fn get_substates_from_network(
        &self,
        ids: Vec<SubstateId>,
    ) -> Result<HashMap<SubstateId, Substate>, SubstateApiError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }

        let verify = self.indexer_trust.is_verify();
        let FetchedSubstates { substates, proofs } = self
            .network_interface
            .get_substates(ids, verify)
            .await
            .map_err(|e| SubstateApiError::NetworkInterfaceError(e.into()))?;
        if verify {
            for (id, substate) in &substates {
                self.verify_read(id, substate.version(), substate.substate_value(), proofs.get(id))
                    .await?;
            }
        }
        Ok(substates)
    }

    pub fn load_dependent_substates(
        &self,
        parents: &[&SubstateId],
    ) -> Result<HashSet<SubstateRequirement>, SubstateApiError> {
        let mut substate_ids = HashSet::with_capacity(parents.len());
        let mut tx = self.store.create_read_tx()?;
        for parent_addr in parents {
            let parent = tx.substates_get(parent_addr)?;
            get_dependent_substates(&mut tx, parent, &mut substate_ids)?;
        }
        Ok(substate_ids)
    }

    #[allow(clippy::too_many_lines)]
    pub async fn locate_dependent_substates(
        &self,
        top_level_substates: &[SubstateId],
        unversioned: bool,
    ) -> Result<HashSet<SubstateRequirement>, SubstateApiError> {
        let mut substate_ids = HashSet::with_capacity(top_level_substates.len());

        for parent_id in top_level_substates {
            match self.store.with_read_tx(|tx| tx.substates_get(parent_id)).optional()? {
                Some(parent) => {
                    debug!(
                        target: LOG_TARGET,
                        "Parent substate {} found in local store, loading dependent substates",
                        parent.substate_id
                    );
                    self.store
                        .with_read_tx(|tx| get_dependent_substates(tx, parent, &mut substate_ids))?;

                    // get_dependent_substates silently skips referenced substates that don't
                    // have their own entry in the wallet store. For components, fetch the
                    // current state from the network and add any that were missed.
                    if parent_id.is_component() {
                        let result = self.fetch_substate_from_network(parent_id, None).await?;
                        if let SubstateValue::Component(data) = &result.substate {
                            let value = IndexedWellKnownTypes::from_value(&data.body.state)?;
                            self.add_component_dependencies(&value, unversioned, &mut substate_ids)
                                .await?;
                        }
                    }
                },
                None => {
                    debug!(
                        target: LOG_TARGET,
                        "Parent substate {} not found in store, requesting dependent substates",
                        parent_id
                    );
                    let ValidatorScanResult {
                        id: substate_id,
                        substate,
                        ..
                    } = self.fetch_substate_from_network(parent_id, None).await?;

                    match &substate {
                        SubstateValue::Component(data) => {
                            let value = IndexedWellKnownTypes::from_value(&data.body.state)?;
                            self.add_component_dependencies(&value, unversioned, &mut substate_ids)
                                .await?;
                        },
                        SubstateValue::Resource(_) => {},
                        SubstateValue::TransactionReceipt(_) => {
                            let addr = substate_id
                                .substate_id()
                                .as_transaction_receipt_address()
                                .ok_or_else(|| {
                                    SubstateApiError::InvalidValidatorNodeResponse(format!(
                                        "Transaction receipt substate and substate ID mismatch! Got {}",
                                        substate_id
                                    ))
                                })?;
                            let tx_receipt_addr = SubstateId::TransactionReceipt(addr);
                            if substate_ids.contains(&tx_receipt_addr) {
                                continue;
                            }
                            if unversioned {
                                substate_ids.insert(tx_receipt_addr.into());
                            } else {
                                // Tx receipts are always v0
                                substate_ids
                                    .insert(SubstateRequirement::versioned(tx_receipt_addr, SubstateVersion::ZERO));
                            }
                        },
                        SubstateValue::Vault(vault) => {
                            let resx_addr = SubstateId::Resource(*vault.resource_address());
                            self.insert_substate_if_absent(&resx_addr, unversioned, &mut substate_ids)
                                .await?;
                        },
                        SubstateValue::NonFungible(_) => {
                            let nft_addr = substate_id.substate_id().as_non_fungible_address().ok_or_else(|| {
                                SubstateApiError::InvalidValidatorNodeResponse(format!(
                                    "NonFungible substate does not have a valid address {}",
                                    substate_id
                                ))
                            })?;

                            let resx_addr = SubstateId::Resource(*nft_addr.resource_address());
                            self.insert_substate_if_absent(&resx_addr, unversioned, &mut substate_ids)
                                .await?;
                        },
                        SubstateValue::ValidatorFeePool(_) => {
                            let resx_addr = SubstateId::Resource(TARI_TOKEN);
                            if substate_ids.contains(&resx_addr) {
                                continue;
                            }
                        },
                        SubstateValue::ClaimedOutputTombstone(_) => {},
                        SubstateValue::Template(_) => {},
                        SubstateValue::Utxo(_) => {
                            let addr = substate_id.substate_id().as_utxo_address().ok_or_else(|| {
                                SubstateApiError::InvalidValidatorNodeResponse(format!(
                                    "Utxo substate does not have a valid address {}",
                                    substate_id
                                ))
                            })?;

                            let resx_addr = SubstateId::Resource(*addr.resource_address());
                            self.insert_substate_if_absent(&resx_addr, unversioned, &mut substate_ids)
                                .await?;
                        },
                        SubstateValue::ConfidentialOutput(_) => {
                            let addr = substate_id
                                .substate_id()
                                .as_confidential_output_address()
                                .ok_or_else(|| {
                                    SubstateApiError::InvalidValidatorNodeResponse(format!(
                                        "ConfidentialOutput substate does not have a valid address {}",
                                        substate_id
                                    ))
                                })?;

                            let resx_addr = SubstateId::Resource(*addr.resource_address());
                            self.insert_substate_if_absent(&resx_addr, unversioned, &mut substate_ids)
                                .await?;
                        },
                    }

                    debug!(
                        target: LOG_TARGET,
                        "Adding substate {} to dependent substates from remote source",
                        substate_id
                    );
                    substate_ids.insert(substate_id.into());
                },
            }
        }

        // `get_dependent_substates` and the parent fetches above carry the version from the wallet's
        // local store / network scan, ignoring the `unversioned` request. When the caller asked for
        // unversioned dependencies, normalise the whole set: a locally-cached version can be stale
        // (e.g. a swap pool reserve advances on every swap), and pinning it makes the indexer reject the
        // input as "down" once the chain moves past it. `SubstateRequirement` hashes by id only, so
        // collapsing to unversioned cannot introduce duplicates.
        if unversioned {
            return Ok(substate_ids
                .into_iter()
                .map(SubstateRequirement::into_unversioned)
                .collect());
        }

        Ok(substate_ids)
    }

    /// Adds referenced substates and vault resource addresses from a component's state.
    async fn add_component_dependencies(
        &self,
        value: &IndexedWellKnownTypes,
        unversioned: bool,
        substate_ids: &mut HashSet<SubstateRequirement>,
    ) -> Result<(), SubstateApiError> {
        for addr in value.referenced_substates() {
            // Component state legitimately holds engine-issued addresses — an `Account` approval is keyed on a
            // spender badge, which is commonly a public-key identity — and those have no substate to fetch or lock.
            if addr.is_virtual() {
                continue;
            }
            self.insert_substate_if_absent(&addr, unversioned, substate_ids).await?;
        }

        // Fetch each vault referenced in the component state and add its resource address
        // as a dependency.
        for vault_id in value.vault_ids() {
            let vault_addr = SubstateId::Vault(*vault_id);
            let result = self.fetch_substate_from_network(&vault_addr, None).await?;
            if let SubstateValue::Vault(vault) = &result.substate {
                let resx_addr = SubstateId::Resource(*vault.resource_address());
                self.insert_substate_if_absent(&resx_addr, unversioned, substate_ids)
                    .await?;
            }
        }

        Ok(())
    }

    /// Inserts a substate ID into the set if it is not already present. If `unversioned` is false,
    /// fetches the current version from the network before inserting.
    async fn insert_substate_if_absent(
        &self,
        substate_id: &SubstateId,
        unversioned: bool,
        substate_ids: &mut HashSet<SubstateRequirement>,
    ) -> Result<(), SubstateApiError> {
        if substate_ids.contains(substate_id) {
            return Ok(());
        }
        if unversioned {
            substate_ids.insert(substate_id.clone().into());
        } else {
            let ValidatorScanResult { id, .. } = self.fetch_substate_from_network(substate_id, None).await?;
            substate_ids.insert(id.into());
        }
        Ok(())
    }

    /// Fetches `address` at `version_hint` or later, or its latest version. Under [`IndexerTrust::Verify`] the substate
    /// must be proven by a committee of a validator set the wallet trusts, at a block no more than
    /// [`MAX_PROOF_AGE_EPOCHS`] before the latest such set, or the fetch fails.
    pub async fn fetch_substate_from_network(
        &self,
        address: &SubstateId,
        version_hint: Option<SubstateVersion>,
    ) -> Result<ValidatorScanResult, SubstateApiError> {
        debug!(
            target: LOG_TARGET,
            "Fetching for substate {} at version {}",
            address,
            version_hint.display()
        );

        let verify = self.indexer_trust.is_verify();
        let resp = self
            .network_interface
            .query_substate(address, version_hint, false, verify)
            .await
            .optional()
            .map_err(|e| SubstateApiError::NetworkInterfaceError(e.into()))?
            .ok_or_else(|| SubstateApiError::SubstateDoesNotExist {
                address: address.clone(),
            })?;
        if let Some(requested) = version_hint &&
            resp.version < requested
        {
            return Err(SubstateApiError::SubstateOlderThanRequested {
                substate_id: address.clone(),
                requested,
                served: resp.version,
            });
        }
        if verify {
            self.verify_read(address, resp.version, &resp.substate, resp.proof.as_ref())
                .await?;
        }

        debug!(
            target: LOG_TARGET,
            "Found substate {} at version {}", address, resp.version
        );
        Ok(ValidatorScanResult {
            id: VersionedSubstateId::new(address.clone(), resp.version),
            substate: resp.substate,
        })
    }

    /// Verifies that `substate_id` at `version` held `value` at the block `proof` is anchored at, and that the block
    /// is recent: no more than [`MAX_PROOF_AGE_EPOCHS`] before the latest validator set the wallet trusts.
    ///
    /// The latest trusted set advances only as far as the checkpoints the indexer serves. An indexer that stops serving
    /// newer checkpoints can therefore keep the wallet on state proven up to the last one it served; no proof shows its
    /// age against the wall clock.
    async fn verify_read(
        &self,
        substate_id: &SubstateId,
        version: SubstateVersion,
        value: &SubstateValue,
        proof: Option<&SubstateProof>,
    ) -> Result<(), SubstateApiError> {
        let proof = proof.ok_or_else(|| SubstateApiError::SubstateUnproven {
            substate_id: substate_id.clone(),
        })?;
        let anchor_epoch = proof.anchor.epoch;
        // A proof anchored before the latest trusted set walks nowhere, so it is judged before its committee is
        // fetched.
        self.check_fresh(substate_id, anchor_epoch)?;
        let committee = self
            .validator_set_api
            .verify_committee(anchor_epoch, proof.anchor.shard_group)
            .await?
            .ok_or_else(|| SubstateApiError::SubstateProvenOutsideTrustedSets {
                substate_id: substate_id.clone(),
                epoch: anchor_epoch,
            })?;
        committee
            .verify_substate(substate_id, version, Some(value), proof)
            .map_err(|source| SubstateApiError::SubstateUnverified {
                substate_id: substate_id.clone(),
                source,
            })?;
        // Verifying the committee walks to the set of the anchor's epoch when that is past the latest trusted set, and
        // the walk can go further.
        self.check_fresh(substate_id, anchor_epoch)
    }

    fn check_fresh(&self, substate_id: &SubstateId, anchor_epoch: Epoch) -> Result<(), SubstateApiError> {
        let Some(latest) = self.validator_set_api.latest()? else {
            return Ok(());
        };
        if anchor_epoch.as_u64() + MAX_PROOF_AGE_EPOCHS < latest.epoch().as_u64() {
            return Err(SubstateApiError::SubstateProofStale {
                substate_id: substate_id.clone(),
                anchor_epoch,
                latest_trusted_epoch: latest.epoch(),
            });
        }
        Ok(())
    }

    pub async fn fetch_resource(&self, address: ResourceAddress) -> Result<Resource, SubstateApiError> {
        if let Some(resource) = self.store.with_read_tx(|tx| tx.resources_get(&address)).optional()? {
            return Ok(resource.into());
        }
        let ValidatorScanResult { substate, .. } = self.fetch_substate_from_network(&address.into(), None).await?;

        let resource = substate
            .as_resource()
            .ok_or_else(|| {
                SubstateApiError::InvalidValidatorNodeResponse(format!("Substate at {} is not a Resource", address))
            })?
            .clone();
        self.store
            .with_write_tx(|tx| tx.resources_upsert(&address, &resource))?;
        Ok(resource)
    }

    pub fn save_root<I: IntoIterator<Item = SubstateId>>(
        &self,
        id: VersionedSubstateIdRef<'_>,
        referenced_substates: I,
    ) -> Result<(), SubstateApiError> {
        self.store.with_write_tx(|tx| {
            let maybe_removed = tx.substates_remove(id.substate_id()).optional()?;
            tx.substates_upsert_root(
                id,
                referenced_substates.into_iter().collect(),
                None,
                maybe_removed.and_then(|s| s.template_address),
            )
        })?;
        Ok(())
    }

    pub fn save_child<I: IntoIterator<Item = SubstateId>>(
        &self,
        parent: &SubstateId,
        child: VersionedSubstateIdRef<'_>,
        referenced_substates: I,
    ) -> Result<(), SubstateApiError> {
        self.store.with_write_tx(|tx| {
            let maybe_substate = tx.substates_remove(child.substate_id()).optional()?;
            tx.substates_upsert_child(
                parent,
                child,
                maybe_substate
                    .into_iter()
                    .flat_map(|s| s.referenced_substates)
                    .chain(referenced_substates)
                    .collect(),
            )
        })?;

        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SubstateApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
    #[error("Network interface error: {0}")]
    NetworkInterfaceError(anyhow::Error),
    #[error("Invalid validator node response: {0}")]
    InvalidValidatorNodeResponse(String),
    #[error("Substate {address} does not exist")]
    SubstateDoesNotExist { address: SubstateId },
    #[error("ValueVisitorError: {0}")]
    ValueVisitorError(#[from] IndexedValueError),
    #[error("The indexer served substate {substate_id} without a proof. Retry, or use another indexer.")]
    SubstateUnproven { substate_id: SubstateId },
    #[error(
        "The indexer proved substate {substate_id} at a block of {epoch}, for which the wallet trusts no validator set"
    )]
    SubstateProvenOutsideTrustedSets { substate_id: SubstateId, epoch: Epoch },
    #[error(
        "The indexer proved substate {substate_id} at a block of {anchor_epoch}, too long before \
         {latest_trusted_epoch} to show its current state. Retry, or use another indexer."
    )]
    SubstateProofStale {
        substate_id: SubstateId,
        anchor_epoch: Epoch,
        latest_trusted_epoch: Epoch,
    },
    #[error("The indexer served substate {substate_id} at v{served}, older than the v{requested} asked for")]
    SubstateOlderThanRequested {
        substate_id: SubstateId,
        requested: SubstateVersion,
        served: SubstateVersion,
    },
    #[error("The indexer's proof of substate {substate_id} does not verify: {source}")]
    SubstateUnverified {
        substate_id: SubstateId,
        source: LightClientError,
    },
    #[error("Validator set error: {0}")]
    ValidatorSet(#[from] ValidatorSetApiError),
}

impl IsNotFoundError for SubstateApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::SubstateDoesNotExist { .. }) ||
            matches!(self, Self::StoreError(e) if e.is_not_found_error())
    }
}

pub struct ValidatorScanResult {
    pub id: VersionedSubstateId,
    pub substate: SubstateValue,
}

fn get_dependent_substates<TTx: WalletStoreReader>(
    tx: &mut TTx,
    parent: SubstateModel,
    substate_ids: &mut HashSet<SubstateRequirement>,
) -> Result<(), WalletStorageError> {
    // TODO: this was done to just quickly get things to work but could cause endless recursion or have other bugs -
    // time should be taken to improve this.
    substate_ids.insert(parent.substate_id.clone().into());
    for child in parent.referenced_substates {
        get_dependent_substates_inner(tx, &child, substate_ids)?;
    }

    let children = tx.substates_get_children(parent.substate_id.substate_id())?;
    for child in children {
        if let Some(addr) = child.substate_id.substate_id().as_non_fungible_address() {
            // Ensure that the associated resource is also included
            substate_ids.insert(SubstateRequirement::unversioned(*addr.resource_address()));
        }
        debug!(
            target: LOG_TARGET,
            "substate {} owned by {}",
            child.substate_id,
            parent.substate_id
        );
        substate_ids.insert(child.substate_id.into());
        for child in child.referenced_substates {
            get_dependent_substates_inner(tx, &child, substate_ids)?;
        }
    }
    Ok(())
}
fn get_dependent_substates_inner<TTx: WalletStoreReader>(
    tx: &mut TTx,
    id: &SubstateId,
    substate_ids: &mut HashSet<SubstateRequirement>,
) -> Result<(), WalletStorageError> {
    let Some(substate) = tx.substates_get(id).optional()? else {
        return Ok(());
    };
    debug!(
        target: LOG_TARGET,
        "Getting dependent substates for {}",
        substate.substate_id,
    );
    substate_ids.insert(substate.substate_id.into());

    for child in substate.referenced_substates {
        if let Some(addr) = child.as_non_fungible_address() {
            debug!(
                target: LOG_TARGET,
                "NonFungible substate {} owned by {}",
                child,
                id
            );
            // Ensure that the associated resource is also included
            substate_ids.insert(SubstateRequirement::unversioned(*addr.resource_address()));
        }

        if substate_ids.contains(&child) {
            continue;
        }

        get_dependent_substates_inner(tx, &child, substate_ids)?;
        debug!(
            target: LOG_TARGET,
            "Child substate {} owned by {}",
            child,
            id
        );
        substate_ids.insert(child.into());
    }
    Ok(())
}
