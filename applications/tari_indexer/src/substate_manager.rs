//  Copyright 2023, The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use ootle_network::Network;
use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_engine_types::{
    Utxo,
    substate::{Substate, SubstateId, SubstateValue},
};
use tari_epoch_manager::service::EpochManagerHandle;
use tari_indexer_client::types::{ListSubstateItem, NonFungibleSubstate, UtxoStateUpdateSet};
use tari_indexer_lib::{
    cached_substate_manager::{CachedSubstateManager, TrustedRootStore},
    error::IndexerError,
};
use tari_ootle_common_types::{
    Epoch,
    ShardGroup,
    StateVersion,
    SubstateRequirementRef,
    optional::IsNotFoundError,
    shard::Shard,
    substate_type::SubstateType,
};
use tari_ootle_p2p::PeerAddress;
use tari_ootle_storage::{StorageError, consensus_models::VerifiedBlockTip};
use tari_template_lib_types::{
    ResourceAddress,
    TemplateAddress,
    UtxoId,
    crypto::{RistrettoPublicKeyBytes, UtxoTag},
};
use tari_validator_node_rpc::client::{SubstateResult, TariValidatorNodeRpcClientFactory};

use crate::{
    storage_sqlite::{SqliteIndexerStore, models::VerifiedStateRoot},
    store::{IndexerStore, IndexerStoreReadTransaction, IndexerStoreReader, IndexerStoreWriteTransaction},
    substate_cache::SqliteSubstateCache,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct SubstateResponse {
    pub id: SubstateId,
    pub version: u64,
    pub substate: SubstateValue,
}

/// A substate returned by a lookup together with whether it was committee-verified. `verified` is
/// false when proof verification is disabled, or when no committee member could supply a proof yet
/// (e.g. nothing has been committed since an epoch change).
#[derive(Debug, Clone)]
pub struct FetchedSubstate {
    pub substate: Substate,
    pub verified: bool,
}

/// Adapts the indexer's sqlite store to [`TrustedRootStore`], which the read path consults to skip
/// commit-proof re-validation on a trusted-root hit and warms with newly-validated roots.
#[derive(Clone)]
struct SqliteTrustedRootStore {
    store: SqliteIndexerStore,
}

impl std::fmt::Debug for SqliteTrustedRootStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteTrustedRootStore").finish_non_exhaustive()
    }
}

#[async_trait]
impl TrustedRootStore for SqliteTrustedRootStore {
    async fn is_trusted(&self, epoch: Epoch, shard_group: ShardGroup, root: FixedHash) -> Result<bool, IndexerError> {
        self.store
            .with_read_tx(move |tx| tx.is_state_root_trusted(epoch, shard_group, &root))
            .await
            .map_err(|e: StorageError| IndexerError::TrustedRootStoreError(e.to_string()))
    }

    async fn record(&self, tip: VerifiedBlockTip) -> Result<(), IndexerError> {
        let root = VerifiedStateRoot::from_verified_tip(&tip);
        self.store
            .with_write_tx(move |tx| tx.upsert_verified_state_root(&root))
            .await
            .map_err(|e: StorageError| IndexerError::TrustedRootStoreError(e.to_string()))
    }
}

#[derive(Debug, Clone)]
pub struct SubstateManager {
    cache_manager:
        CachedSubstateManager<EpochManagerHandle<PeerAddress>, TariValidatorNodeRpcClientFactory, SqliteSubstateCache>,
    substate_store: SqliteIndexerStore,
}

impl SubstateManager {
    pub fn new(
        network: Network,
        substate_store: SqliteIndexerStore,
        epoch_manager: EpochManagerHandle<PeerAddress>,
        validator_node_client_factory: TariValidatorNodeRpcClientFactory,
        substate_cache: SqliteSubstateCache,
    ) -> Self {
        // Consulted only when proof verification is enabled (the dry-run manager has it off, so its
        // copy is inert). Lets a read skip re-validating a served commit proof when its root is
        // already trusted, and is warmed with newly-validated roots.
        let trusted_root_store = Arc::new(SqliteTrustedRootStore {
            store: substate_store.clone(),
        });
        let cached_substates = CachedSubstateManager::new(
            network,
            epoch_manager.clone(),
            validator_node_client_factory.clone(),
            substate_cache,
        )
        .with_trusted_root_store(trusted_root_store);

        Self {
            cache_manager: cached_substates,
            substate_store,
        }
    }

    pub fn with_cache_ttl(mut self, ttl: Duration) -> Self {
        self.cache_manager = self.cache_manager.with_cache_ttl(ttl);
        self
    }

    pub fn with_negative_cache_ttl(mut self, ttl: Duration) -> Self {
        self.cache_manager = self.cache_manager.with_negative_cache_ttl(ttl);
        self
    }

    pub fn with_substate_proof_verification(mut self, enabled: bool) -> Self {
        self.cache_manager = self.cache_manager.with_substate_proof_verification(enabled);
        self
    }

    /// Whether substates served by this manager are verified against the shard group committee.
    pub fn verifies_substates(&self) -> bool {
        self.cache_manager.verifies_substates()
    }

    #[cfg(feature = "metrics")]
    pub fn with_metrics(self, registry_mut: &mut prometheus_client::registry::Registry) -> Self {
        let cached_substates = self.cache_manager.with_metrics(registry_mut);
        Self {
            cache_manager: cached_substates,
            substate_store: self.substate_store,
        }
    }

    pub async fn get_stored_substates_by_filters(
        &self,
        by_id: Option<&SubstateId>,
        filter_by_type: Option<SubstateType>,
        filter_by_template: Option<TemplateAddress>,
        limit: Option<u64>,
        offset: Option<u64>,
    ) -> Result<Vec<ListSubstateItem>, SubstateManagerError> {
        let by_id = by_id.cloned();
        let substates = self
            .substate_store
            .with_read_tx(move |tx| {
                tx.list_substates(by_id.as_ref(), filter_by_type, filter_by_template, limit, offset)
            })
            .await?;
        Ok(substates)
    }

    pub async fn get_utxo_updates(
        &self,
        resource_address: ResourceAddress,
        from_epoch: Epoch,
        shard: Shard,
        from_state_version: StateVersion,
        unspent_only: bool,
        limit: u32,
    ) -> Result<UtxoStateUpdateSet, SubstateManagerError> {
        let updates = self
            .substate_store
            .with_read_tx(move |tx| {
                tx.utxos_get_updates(
                    resource_address,
                    from_epoch,
                    shard,
                    from_state_version,
                    unspent_only,
                    limit,
                )
            })
            .await?;
        Ok(updates)
    }

    pub async fn get_max_state_version(
        &self,
        resource_address: &ResourceAddress,
        shard: Shard,
    ) -> Result<StateVersion, SubstateManagerError> {
        let resource_address = *resource_address;
        let max_version = self
            .substate_store
            .with_read_tx(move |tx| tx.utxos_get_max_state_version(resource_address, shard))
            .await?;
        Ok(max_version)
    }

    pub async fn get_unspent_utxos(
        &self,
        resource_address: &ResourceAddress,
        public_nonce_and_tag: &[(UtxoTag, RistrettoPublicKeyBytes)],
    ) -> Result<Vec<(UtxoId, Utxo)>, SubstateManagerError> {
        let resource_address = *resource_address;
        let public_nonce_and_tag = public_nonce_and_tag.to_vec();
        let utxos = self
            .substate_store
            .with_read_tx(move |tx| {
                tx.utxos_get_unspent_by_public_nonce_and_tag(&resource_address, &public_nonce_and_tag)
            })
            .await?;
        Ok(utxos)
    }

    pub async fn list_utxos(
        &self,
        resource_address: &ResourceAddress,
        from_id: Option<UtxoId>,
        limit: u32,
    ) -> Result<Vec<(UtxoId, Utxo)>, SubstateManagerError> {
        let resource_address = *resource_address;
        let utxos = self
            .substate_store
            .with_read_tx(move |tx| tx.utxos_list(&resource_address, from_id, limit))
            .await?;
        Ok(utxos)
    }

    pub async fn get_substate(&self, req: SubstateRequirementRef<'_>) -> Result<Substate, SubstateManagerError> {
        let lookup_result = self
            .cache_manager
            .get_substate(req.substate_id(), req.version())
            .await?;
        match lookup_result.result {
            SubstateResult::Up { substate } => Ok(*substate),
            SubstateResult::Down { version } => Err(SubstateManagerError::InputSubstateIsDown {
                substate_id: req.substate_id().clone(),
                version,
            }),
            SubstateResult::DoesNotExist => Err(SubstateManagerError::InputSubstateDoesNotExist {
                substate_id: req.substate_id().clone(),
            }),
        }
    }

    pub async fn get_substates<'a, I: IntoIterator<Item = SubstateRequirementRef<'a>>>(
        &self,
        substate_req: I,
    ) -> Result<HashMap<SubstateId, FetchedSubstate>, SubstateManagerError> {
        let substate_req = substate_req.into_iter().collect::<HashSet<_>>();
        let mut results = HashMap::with_capacity(substate_req.len());

        let mut found_in_cache = HashSet::new();

        for req in &substate_req {
            if let Some(version) = req.version() &&
                let Some(substate) = self.get_substate_from_db(req.substate_id(), Some(version)).await?
            {
                found_in_cache.insert(*req);
                results.insert(req.substate_id().clone(), FetchedSubstate {
                    substate: Substate::new(substate.version, substate.substate),
                    // Locally-stored substates were verified when ingested iff verification is on.
                    verified: self.cache_manager.verifies_substates(),
                });
            }
        }

        // TODO(perf): consider batch fetching from cache and validator nodes
        for req in substate_req {
            if found_in_cache.contains(&req) {
                continue;
            }
            let lookup_result = self
                .cache_manager
                .get_substate(req.substate_id(), req.version())
                .await?;
            match lookup_result.result {
                SubstateResult::DoesNotExist => {
                    // Skip, does not exist
                },
                SubstateResult::Up { substate } => {
                    results.insert(req.substate_id().clone(), FetchedSubstate {
                        substate: Substate::new(substate.version(), substate.into_substate_value()),
                        verified: lookup_result.verified,
                    });
                },
                SubstateResult::Down { version } => {
                    return Err(SubstateManagerError::InputSubstateIsDown {
                        substate_id: req.substate_id().clone(),
                        version,
                    });
                },
            }
        }
        Ok(results)
    }

    pub async fn get_cached_substates(
        &self,
        substates: &[SubstateId],
    ) -> Result<HashMap<SubstateId, Substate>, SubstateManagerError> {
        let mut substate_set = substates.iter().collect::<HashSet<_>>();

        let substates_arg = substates.to_vec();
        let mut substates = self
            .substate_store
            .with_read_tx(move |tx| tx.get_substates(&substates_arg))
            .await?;

        for k in substates.keys() {
            substate_set.remove(k);
        }

        let cached = self
            .cache_manager
            .get_cached_substates(substate_set.into_iter())
            .await?;
        for (id, substate) in cached {
            let Some(substate) = substate else {
                continue;
            };
            let Some(substate) = substate.substate_result.into_up() else {
                continue;
            };
            substates.insert(id.clone(), substate);
        }

        Ok(substates)
    }

    pub async fn fetch_and_cache_substates(
        &self,
        substate_ids: &[SubstateId],
    ) -> Result<HashMap<SubstateId, Substate>, SubstateManagerError> {
        let result = self.cache_manager.fetch_and_cache_substates(substate_ids).await?;
        Ok(result)
    }

    async fn get_substate_from_db(
        &self,
        substate_address: &SubstateId,
        version: Option<u64>,
    ) -> Result<Option<SubstateResponse>, SubstateManagerError> {
        let substate_address = substate_address.clone();
        let row = self
            .substate_store
            .with_read_tx(move |tx| tx.get_substate(&substate_address, version))
            .await?;
        match row {
            Some(row) => Ok(Some(row.try_into()?)),
            None => Ok(None),
        }
    }

    pub async fn get_non_fungibles_by_resource_address(
        &self,
        address: ResourceAddress,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<NonFungibleSubstate>, SubstateManagerError> {
        let nfts = self
            .substate_store
            .with_read_tx(move |tx| tx.get_non_fungibles_by_resource_address(address, limit, offset))
            .await?;
        Ok(nfts)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum SubstateManagerError {
    #[error("Indexer error: {0}")]
    IndexerError(#[from] IndexerError),
    #[error("Storage error: {0}")]
    StorageError(#[from] StorageError),
    #[error("Input substate {substate_id} (v{version}) is down")]
    InputSubstateIsDown { substate_id: SubstateId, version: u64 },
    #[error("Input substate {substate_id} does not exist")]
    InputSubstateDoesNotExist { substate_id: SubstateId },
}

impl IsNotFoundError for SubstateManagerError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::InputSubstateDoesNotExist { .. }) ||
            matches!(self, Self::StorageError(e) if e.is_not_found_error())
    }
}
