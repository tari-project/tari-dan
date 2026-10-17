//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use tari_indexer_client::types::MAX_SUBSTATES_PER_FETCH;
use tari_ootle_common_types::{
    InputDeclaration,
    engine_types::{
        indexed_value::IndexedValueError,
        substate::{SubstateId, SubstateValue},
    },
    substate_type::SubstateType,
};
use tari_ootle_transaction::UnsignedTransaction;
use tari_template_lib_types::{ComponentAddress, constants::TARI_TOKEN};
use tracing::debug;

use crate::{
    macros::_macro_exports::ResourceAddress,
    provider::{VerifyError, WantInput, verifier::IndexerVerifier},
};

const LOG_TARGET: &str = "ootle_rs::wallet::provider::input_resolver";

#[derive(Debug, thiserror::Error)]
pub enum TransactionInputResolverError {
    #[error("Failed to resolve transaction input: {0}")]
    IndexerClientError(#[from] tari_indexer_client::error::IndexerRestClientError),
    #[error(transparent)]
    Unverified(#[from] VerifyError),
    #[error("Indexed value error: {0}")]
    IndexedValueError(#[from] IndexedValueError),
    #[error("Required substate {substate_id} not found: {details}")]
    RequiredSubstateNotFound { substate_id: SubstateId, details: String },
    #[error("Unexpected substate type. Expected: {expected}, Found: {found}")]
    UnexpectedSubstateType {
        expected: SubstateType,
        found: SubstateType,
    },
}

pub struct TransactionInputResolver {
    verifier: Arc<IndexerVerifier>,
    cache: HashMap<SubstateId, Option<SubstateValue>>,
}

impl TransactionInputResolver {
    pub(crate) fn new(verifier: Arc<IndexerVerifier>) -> Self {
        Self {
            verifier,
            cache: HashMap::new(),
        }
    }

    pub async fn resolve_inputs(
        &mut self,
        tx_mut: &mut UnsignedTransaction,
        want_list: &HashSet<WantInput>,
    ) -> Result<(), TransactionInputResolverError> {
        let mut wants = want_list.iter().map(|i| (i, false)).collect::<HashMap<_, _>>();

        let mut substates_to_cache = Vec::new();
        let mut num_unsatisfied = want_list.len();
        loop {
            for (want, is_satisfied) in &mut wants {
                if *is_satisfied {
                    continue;
                }

                if self.resolve_want_input(want, tx_mut, &mut substates_to_cache)? {
                    *is_satisfied = true;
                    num_unsatisfied -= 1;
                }
            }

            // Cheaper than `wants.iter().all(|(_, s)| **s)`
            if num_unsatisfied == 0 {
                break;
            }

            // Populate the cache and then do another pass
            self.cache_substates(&mut substates_to_cache).await?;
        }

        Ok(())
    }

    fn resolve_want_input(
        &mut self,
        want: &WantInput,
        tx_mut: &mut UnsignedTransaction,
        substates_to_cache: &mut Vec<SubstateId>,
    ) -> Result<bool, TransactionInputResolverError> {
        match want {
            WantInput::VaultForResource {
                component_address,
                resource_address,
                required,
            } => self.resolve_vault_for_resource(
                tx_mut,
                substates_to_cache,
                component_address,
                resource_address,
                *required,
            ),
            WantInput::SpecificSubstate { substate_id, required } => {
                self.resolve_specific_substate(tx_mut, substates_to_cache, substate_id, *required)
            },
            WantInput::AllComponentVaults { component_address } => {
                self.resolve_all_component_vaults(tx_mut, substates_to_cache, component_address)
            },
        }
    }

    fn resolve_specific_substate(
        &mut self,
        tx_mut: &mut UnsignedTransaction,
        substates_to_cache: &mut Vec<SubstateId>,
        substate_id: &SubstateId,
        required: bool,
    ) -> Result<bool, TransactionInputResolverError> {
        // The specific substate is required, so we add it without checking that it actually exists.
        // We _could_ first check that it exists and if not error out here, but validators will reject the transaction
        // in this case anyway, so this saves queries to the indexer (and VNs) in that case. This may be an
        // incorrect trade-off.
        if required {
            tx_mut.add_input(InputDeclaration::write(substate_id.clone()));
            return Ok(true);
        }

        match self.cache.get(substate_id) {
            Some(Some(_)) => {
                tx_mut.add_input(InputDeclaration::write(substate_id.clone()));
                Ok(true)
            },
            Some(None) => Ok(true),
            None => {
                debug!(
                    target: LOG_TARGET,
                    "Will try to cache substate {} to satisfy SubstateIfExists",
                    substate_id
                );
                substates_to_cache.push(substate_id.clone());
                Ok(false)
            },
        }
    }

    fn resolve_vault_for_resource(
        &mut self,
        tx_mut: &mut UnsignedTransaction,
        substates_to_cache: &mut Vec<SubstateId>,
        component_address: &ComponentAddress,
        resource_address: &ResourceAddress,
        required: bool,
    ) -> Result<bool, TransactionInputResolverError> {
        let mut is_satisfied = false;
        let component_substate_id = SubstateId::Component(*component_address);
        match self.cache.get(&component_substate_id) {
            Some(Some(SubstateValue::Component(data))) => {
                let component_state = data.body.to_indexed_well_known_types()?;
                debug!(
                    target: LOG_TARGET,
                    "Checking {} vault(s) for resource {} in component {}",
                    component_state.vault_ids().len(),
                    resource_address,
                    component_address
                );
                let mut have_all_vaults = true;
                for vault_id in component_state
                    .vault_ids()
                    .iter()
                    .map(|vault_id| SubstateId::Vault(*vault_id))
                {
                    match self.cache.get(&vault_id) {
                        Some(Some(vault)) => {
                            let vault = vault.as_vault().ok_or_else(|| {
                                TransactionInputResolverError::UnexpectedSubstateType {
                                    expected: SubstateType::Vault,
                                    found: SubstateType::from(vault),
                                }
                            })?;
                            if vault.resource_address() == resource_address {
                                // Found the vault for the specified resource
                                tx_mut.add_input(InputDeclaration::write(vault_id));
                                if *resource_address != TARI_TOKEN {
                                    tx_mut.add_input(InputDeclaration::read(*resource_address));
                                }
                                is_satisfied = true;
                            }
                        },
                        Some(None) => {
                            // Continue looking at other vaults
                        },
                        None => {
                            have_all_vaults = false;
                            // We'll need to find the vault for the specified resource
                            debug!(
                                target: LOG_TARGET,
                                "Will try to cache vault substate {} to satisfy vault for resource {}",
                                vault_id,
                                resource_address
                            );
                            substates_to_cache.push(vault_id);
                        },
                    }
                }

                if !is_satisfied && have_all_vaults {
                    if required {
                        // Error if we didn't satisfy the want after checking all vaults
                        return Err(TransactionInputResolverError::RequiredSubstateNotFound {
                            substate_id: component_substate_id,
                            details: format!(
                                "Vault for resource {resource_address} in component {component_address} does not exist"
                            ),
                        });
                    } else {
                        // Not required, so we're satisfied even though we didn't find it
                        is_satisfied = true;
                    }
                } else {
                    // Need to cache more vaults
                }
            },
            Some(Some(found)) => {
                // Should never happen
                return Err(TransactionInputResolverError::UnexpectedSubstateType {
                    expected: SubstateType::Component,
                    found: SubstateType::from(found),
                });
            },
            Some(None) if required => {
                return Err(TransactionInputResolverError::RequiredSubstateNotFound {
                    substate_id: component_substate_id,
                    details: "Component does not exist".to_string(),
                });
            },
            Some(None) => {
                // Substate not found but not required
                is_satisfied = true;
            },
            None => {
                debug!(
                    target: LOG_TARGET,
                    "Will try to cache component substate {} to satisfy vault for resource {}",
                    component_address,
                    resource_address
                );
                substates_to_cache.push(SubstateId::Component(*component_address));
            },
        }
        Ok(is_satisfied)
    }

    fn resolve_all_component_vaults(
        &mut self,
        tx_mut: &mut UnsignedTransaction,
        substates_to_cache: &mut Vec<SubstateId>,
        component_address: &ComponentAddress,
    ) -> Result<bool, TransactionInputResolverError> {
        let component_substate_id = SubstateId::Component(*component_address);
        match self.cache.get(&component_substate_id) {
            Some(Some(SubstateValue::Component(data))) => {
                let component_state = data.body.to_indexed_well_known_types()?;
                let vault_ids = component_state
                    .vault_ids()
                    .iter()
                    .map(|vault_id| SubstateId::Vault(*vault_id));

                debug!(
                    target: LOG_TARGET,
                    "Discovering vault(s) in component {}",
                    component_address
                );

                let mut all_cached = true;
                for vault_id in vault_ids {
                    match self.cache.get(&vault_id) {
                        Some(Some(vault)) => {
                            let vault = vault.as_vault().ok_or_else(|| {
                                TransactionInputResolverError::UnexpectedSubstateType {
                                    expected: SubstateType::Vault,
                                    found: SubstateType::from(vault),
                                }
                            })?;
                            tx_mut.add_input(InputDeclaration::write(vault_id));
                            if *vault.resource_address() != TARI_TOKEN {
                                tx_mut.add_input(InputDeclaration::write(*vault.resource_address()));
                            }
                        },
                        Some(None) => {
                            // The vault ID came from the component's state but the indexer
                            // reports it as missing — this indicates an inconsistent view.
                            return Err(TransactionInputResolverError::RequiredSubstateNotFound {
                                substate_id: vault_id,
                                details: format!(
                                    "Vault referenced by component {} was not found by the indexer",
                                    component_address
                                ),
                            });
                        },
                        None => {
                            all_cached = false;
                            debug!(
                                target: LOG_TARGET,
                                "Will try to cache vault substate {} for AllComponentVaults discovery",
                                vault_id,
                            );
                            substates_to_cache.push(vault_id);
                        },
                    }
                }

                Ok(all_cached)
            },
            Some(Some(found)) => Err(TransactionInputResolverError::UnexpectedSubstateType {
                expected: SubstateType::Component,
                found: SubstateType::from(found),
            }),
            Some(None) => Err(TransactionInputResolverError::RequiredSubstateNotFound {
                substate_id: component_substate_id,
                details: "Component does not exist".to_string(),
            }),
            None => {
                debug!(
                    target: LOG_TARGET,
                    "Will try to cache component substate {} for AllComponentVaults discovery",
                    component_address,
                );
                substates_to_cache.push(component_substate_id);
                Ok(false)
            },
        }
    }

    async fn cache_substates(
        &mut self,
        substates_to_cache: &mut Vec<SubstateId>,
    ) -> Result<(), TransactionInputResolverError> {
        for batch in substates_to_cache.chunks(MAX_SUBSTATES_PER_FETCH) {
            let substates = self.verifier.fetch_substates(batch).await?;

            self.cache.extend(
                substates
                    .into_iter()
                    .map(|(id, substate)| (id, Some(substate.into_substate_value()))),
            );
            // Add any not found substates to the cache as None (not found)
            for id in batch {
                if !self.cache.contains_key(id) {
                    self.cache.insert(id.clone(), None);
                }
            }
        }

        substates_to_cache.clear();

        Ok(())
    }
}
