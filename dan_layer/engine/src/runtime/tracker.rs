//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{
    cmp::min,
    collections::{BTreeSet, HashMap},
    convert::TryFrom,
    mem,
    sync::{Arc, Mutex, RwLock},
};

use log::debug;
use tari_common_types::types::PublicKey;
use tari_dan_common_types::{optional::Optional, Epoch, NodeAddressable};
use tari_engine_types::{
    bucket::Bucket,
    commit_result::{RejectReason, TransactionResult},
    component::{ComponentBody, ComponentHeader},
    confidential::UnclaimedConfidentialOutput,
    events::Event,
    fee_claim::FeeClaimAddress,
    fees::{FeeReceipt, FeeSource},
    indexed_value::IndexedValue,
    logs::LogEntry,
    non_fungible::NonFungibleContainer,
    non_fungible_index::NonFungibleIndex,
    proof::{LockedResource, Proof},
    resource::Resource,
    resource_container::ResourceContainer,
    substate::{Substate, SubstateAddress, SubstateDiff, SubstateValue},
    transaction_receipt::TransactionReceipt,
    vault::Vault,
    TemplateAddress,
};
use tari_template_lib::{
    args::MintArg,
    auth::{ComponentAccessRules, OwnerRule, ResourceAccessRules},
    constants::CONFIDENTIAL_TARI_RESOURCE_ADDRESS,
    crypto::RistrettoPublicKeyBytes,
    models::{
        Amount,
        BucketId,
        ComponentAddress,
        Metadata,
        NonFungibleAddress,
        NonFungibleIndexAddress,
        ProofId,
        ResourceAddress,
        UnclaimedConfidentialOutputAddress,
        VaultId,
    },
    resource::ResourceType,
    Hash,
};
use tari_transaction::id_provider::IdProvider;

use crate::{
    runtime::{
        fee_state::FeeState,
        tracker_auth::Authorization,
        working_state::WorkingState,
        workspace::Workspace,
        AuthorizationScope,
        RuntimeError,
        TransactionCommitError,
        VirtualSubstates,
    },
    state_store::{memory::MemoryStateStore, AtomicDb, StateReader},
};

const LOG_TARGET: &str = "tari::dan::engine::runtime::state_tracker";

pub struct FinalizeTracker {
    pub result: TransactionResult,
    pub events: Vec<Event>,
    pub fee_receipt: FeeReceipt,
    pub logs: Vec<LogEntry>,
}

#[derive(Debug, Clone)]
pub struct StateTracker {
    working_state: Arc<RwLock<WorkingState>>,
    fee_state: Arc<RwLock<FeeState>>,
    id_provider: IdProvider,
    fee_checkpoint: Arc<Mutex<Option<WorkingState>>>,
}

#[derive(Debug, Clone)]
pub struct RuntimeState {
    pub template_name: String,
    pub template_address: TemplateAddress,
    pub transaction_signer_public_key: RistrettoPublicKeyBytes,
    pub component_address: Option<ComponentAddress>,
    pub recursion_depth: usize,
    pub max_recursion_depth: usize,
}

impl StateTracker {
    pub fn new(
        state_store: MemoryStateStore,
        id_provider: IdProvider,
        virtual_substates: VirtualSubstates,
        auth_scope: AuthorizationScope,
    ) -> Self {
        Self {
            working_state: Arc::new(RwLock::new(WorkingState::new(
                state_store,
                virtual_substates,
                auth_scope,
            ))),
            fee_state: Arc::new(RwLock::new(FeeState::new())),
            id_provider,
            fee_checkpoint: Arc::new(Mutex::new(None)),
        }
    }

    pub fn get_current_epoch(&self) -> Result<Epoch, RuntimeError> {
        self.read_with(|state| state.get_current_epoch())
    }

    pub fn add_event(&self, event: Event) {
        self.write_with(|state| state.events.push(event))
    }

    pub fn add_log(&self, log: LogEntry) {
        self.write_with(|state| state.logs.push(log));
    }

    pub fn take_events(&self) -> Vec<Event> {
        self.write_with(|state| mem::take(&mut state.events))
    }

    pub fn take_logs(&self) -> Vec<LogEntry> {
        self.write_with(|state| mem::take(&mut state.logs))
    }

    fn check_amount(&self, amount: Amount) -> Result<(), RuntimeError> {
        if amount.is_negative() {
            return Err(RuntimeError::InvalidAmount {
                amount,
                reason: "Amount must be positive".to_string(),
            });
        }
        Ok(())
    }

    pub fn get_template_address(&self) -> Result<TemplateAddress, RuntimeError> {
        self.with_runtime_state(|s| Ok(s.template_address))
    }

    pub fn new_resource(
        &self,
        resource_type: ResourceType,
        owner_rule: OwnerRule,
        access_rules: ResourceAccessRules,
        metadata: Metadata,
    ) -> Result<ResourceAddress, RuntimeError> {
        self.write_with(|state| {
            let runtime_state = state.runtime_state()?;
            let resource_address = self.id_provider.new_resource_address()?;

            let resource = Resource::new(
                resource_type,
                runtime_state.transaction_signer_public_key,
                owner_rule,
                access_rules,
                metadata,
            );

            state.new_resources.insert(resource_address, resource);

            Ok(resource_address)
        })
    }

    pub fn mint_resource(
        &self,
        resource_address: ResourceAddress,
        mint_arg: MintArg,
    ) -> Result<BucketId, RuntimeError> {
        let resource_container = self.write_with(|state| {
            let resource_container = match mint_arg {
                MintArg::Fungible { amount } => {
                    self.check_amount(amount)?;
                    debug!(
                        target: LOG_TARGET,
                        "Minting {} fungible tokens on resource: {}", amount, resource_address
                    );

                    ResourceContainer::fungible(resource_address, amount)
                },
                MintArg::NonFungible { tokens } => {
                    debug!(
                        target: LOG_TARGET,
                        "Minting {} NFT token(s) on resource: {}",
                        tokens.len(),
                        resource_address
                    );
                    let mut token_ids = BTreeSet::new();
                    let resource = state.get_resource(&resource_address)?;
                    let mut index = resource
                        .total_supply()
                        .as_u64_checked()
                        .ok_or(RuntimeError::InvalidAmount {
                            amount: resource.total_supply(),
                            reason: "Could not convert to u64".to_owned(),
                        })?;
                    for (id, (data, mut_data)) in tokens {
                        let nft_address = NonFungibleAddress::new(resource_address, id.clone());
                        if state.get_non_fungible(&nft_address).optional()?.is_some() {
                            return Err(RuntimeError::DuplicateNonFungibleId {
                                token_id: nft_address.id().clone(),
                            });
                        }
                        state
                            .new_non_fungibles
                            .insert(nft_address.clone(), NonFungibleContainer::new(data, mut_data));
                        if !token_ids.insert(id.clone()) {
                            return Err(RuntimeError::DuplicateNonFungibleId { token_id: id });
                        }

                        // for each new nft we also create an index to be allow resource scanning
                        let index_address = NonFungibleIndexAddress::new(resource_address, index);
                        index += 1;
                        let nft_index = NonFungibleIndex::new(nft_address);
                        state.new_non_fungible_indexes.insert(index_address, nft_index);
                    }

                    ResourceContainer::non_fungible(resource_address, token_ids)
                },
                MintArg::Confidential { proof } => {
                    debug!(
                        target: LOG_TARGET,
                        "Minting confidential tokens on resource: {}", resource_address
                    );
                    ResourceContainer::validate_confidential_mint(resource_address, *proof)?
                },
            };

            // Increase the total supply, this also validates that the resource already exists.
            state.borrow_resource_mut(&resource_address, |resource| {
                resource.increase_total_supply(resource_container.amount())
            })?;

            Ok(resource_container)
        })?;

        let bucket = self.new_bucket(resource_container)?;
        Ok(bucket)
    }

    pub fn resource_exists(&self, resource_address: &ResourceAddress) -> Result<bool, RuntimeError> {
        self.read_with(|state| {
            Ok(state.new_resources.contains_key(resource_address) ||
                state.substate_exists_in_state_store(*resource_address)?)
        })
    }

    pub fn get_resource(&self, address: &ResourceAddress) -> Result<Resource, RuntimeError> {
        self.read_with(|state| state.get_resource(address))
    }

    pub fn get_non_fungible(&self, address: &NonFungibleAddress) -> Result<NonFungibleContainer, RuntimeError> {
        self.read_with(|state| state.get_non_fungible(address))
    }

    pub fn set_non_fungible_data(&self, address: &NonFungibleAddress, data: Vec<u8>) -> Result<(), RuntimeError> {
        self.write_with(|state| {
            state.with_non_fungible_mut(address, move |nft| {
                let contents = nft.contents_mut().ok_or(RuntimeError::InvalidOpNonFungibleBurnt {
                    op: "UpdateNonFungibleData",
                    resource_address: *address.resource_address(),
                    nf_id: address.id().clone(),
                })?;
                contents.set_mutable_data(data);
                Ok(())
            })
        })
    }

    pub fn set_resource_access_rules(
        &self,
        address: &ResourceAddress,
        access_rules: ResourceAccessRules,
    ) -> Result<(), RuntimeError> {
        self.write_with(|state| {
            state.borrow_resource_mut(address, move |resource| {
                resource.set_access_rules(access_rules);
            })
        })
    }

    pub fn new_bucket(&self, resource: ResourceContainer) -> Result<BucketId, RuntimeError> {
        self.write_with(|state| {
            let bucket_id = self.id_provider.new_bucket_id();
            debug!(target: LOG_TARGET, "New bucket: {}", bucket_id);
            let bucket = Bucket::new(bucket_id, resource);
            state.buckets.insert(bucket_id, bucket);
            Ok(bucket_id)
        })
    }

    pub fn new_empty_bucket(
        &self,
        resource_address: ResourceAddress,
        resource_type: ResourceType,
    ) -> Result<BucketId, RuntimeError> {
        self.write_with(|state| {
            let bucket_id = self.id_provider.new_bucket_id();
            debug!(
                target: LOG_TARGET,
                "New bucket {} for resource {} {:?}", bucket_id, resource_address, resource_type
            );
            let new_state = match resource_type {
                ResourceType::Fungible => ResourceContainer::fungible(resource_address, Amount::zero()),
                ResourceType::NonFungible => ResourceContainer::non_fungible(resource_address, BTreeSet::new()),
                ResourceType::Confidential => ResourceContainer::confidential(resource_address, None, Amount::zero()),
            };
            let bucket = Bucket::new(bucket_id, new_state);
            state.buckets.insert(bucket_id, bucket);
            Ok(bucket_id)
        })
    }

    pub fn take_bucket(&self, bucket_id: BucketId) -> Result<Bucket, RuntimeError> {
        self.write_with(|state| state.take_bucket(bucket_id))
    }

    pub fn list_buckets(&self) -> Vec<BucketId> {
        self.read_with(|state| state.buckets.keys().copied().collect())
    }

    pub fn borrow_bucket<F: FnOnce(&Bucket) -> R, R>(&self, bucket_id: BucketId, f: F) -> Result<R, RuntimeError> {
        self.read_with(|state| {
            state
                .buckets
                .get(&bucket_id)
                .ok_or(RuntimeError::BucketNotFound { bucket_id })
                .map(f)
        })
    }

    pub fn borrow_bucket_mut<F: FnOnce(&mut Bucket) -> R, R>(
        &self,
        bucket_id: BucketId,
        f: F,
    ) -> Result<R, RuntimeError> {
        self.write_with(|state| {
            state
                .buckets
                .get_mut(&bucket_id)
                .ok_or(RuntimeError::BucketNotFound { bucket_id })
                .map(f)
        })
    }

    pub fn bucket_exists(&self, bucket_id: &BucketId) -> bool {
        self.read_with(|state| state.buckets.contains_key(bucket_id))
    }

    pub fn with_bucket_mut<R, F: FnOnce(&mut Bucket) -> R>(
        &self,
        bucket_id: BucketId,
        callback: F,
    ) -> Result<R, RuntimeError> {
        self.write_with(|state| {
            let bucket = state
                .buckets
                .get_mut(&bucket_id)
                .ok_or(RuntimeError::BucketNotFound { bucket_id })?;
            Ok(callback(bucket))
        })
    }

    pub fn burn_bucket(&self, bucket_id: BucketId) -> Result<(), RuntimeError> {
        self.write_with(|state| {
            let bucket = state.take_bucket(bucket_id)?;
            if bucket.amount().is_zero() {
                return Ok(());
            }
            let resource_address = *bucket.resource_address();
            let burnt_amount = bucket.amount();
            let mut resource = state.get_resource(&resource_address)?;
            // Burn Non-fungibles (if resource is nf). Fungibles are burnt by removing the bucket from the tracker state
            // and not depositing it.
            for token_id in bucket.into_non_fungible_ids().into_iter().flatten() {
                let address = NonFungibleAddress::new(resource_address, token_id);
                let mut nft = state.get_non_fungible(&address)?;

                if nft.is_burnt() {
                    return Err(RuntimeError::InvalidOpNonFungibleBurnt {
                        op: "burn_bucket",
                        resource_address,
                        nf_id: address.id().clone(),
                    });
                }
                nft.burn();
                state.new_non_fungibles.insert(address, nft);
            }

            resource.decrease_total_supply(burnt_amount);
            state.new_resources.insert(resource_address, resource);

            Ok(())
        })
    }

    pub fn new_proof(&self, locked: LockedResource) -> Result<ProofId, RuntimeError> {
        self.write_with(|state| {
            let proof_id = self.id_provider.new_proof_id();
            debug!(target: LOG_TARGET, "New proof: {}", proof_id);
            state.proofs.insert(proof_id, Proof::new(locked));
            Ok(proof_id)
        })
    }

    pub fn proof_exists(&self, proof_id: &ProofId) -> bool {
        self.read_with(|state| state.proofs.contains_key(proof_id))
    }

    pub fn drop_proof(&self, proof_id: ProofId) -> Result<(), RuntimeError> {
        self.write_with(|state| {
            debug!(target: LOG_TARGET, "Drop proof: {}", proof_id);
            state.drop_proof(proof_id)
        })
    }

    pub fn borrow_proof<F: FnOnce(&Proof) -> R, R>(&self, proof_id: &ProofId, f: F) -> Result<R, RuntimeError> {
        self.read_with(|state| {
            state
                .proofs
                .get(proof_id)
                .ok_or(RuntimeError::ProofNotFound { proof_id: *proof_id })
                .map(f)
        })
    }

    pub fn add_proof_to_auth_scope(&self, proof_id: ProofId) -> Result<(), RuntimeError> {
        self.write_with(|state| {
            if !state.proofs.contains_key(&proof_id) {
                return Err(RuntimeError::ProofNotFound { proof_id });
            }
            state.current_auth_scope.add_proof(proof_id);
            Ok(())
        })
    }

    pub fn remove_proof_from_auth_scope(&self, proof_id: &ProofId) -> Result<(), RuntimeError> {
        self.write_with(|state| {
            if !state.proofs.contains_key(proof_id) {
                return Err(RuntimeError::ProofNotFound { proof_id: *proof_id });
            }
            state.current_auth_scope.remove_proof(proof_id);
            Ok(())
        })
    }

    pub fn take_unclaimed_confidential_output(
        &self,
        address: UnclaimedConfidentialOutputAddress,
    ) -> Result<UnclaimedConfidentialOutput, RuntimeError> {
        self.write_with(|state| {
            let output = state.get_unclaimed_confidential_commitment(&address)?;
            state.claim_confidential_output(&address)?;
            Ok(output)
        })
    }

    pub fn claim_fee(&self, epoch: Epoch, validator_public_key: PublicKey) -> Result<ResourceContainer, RuntimeError> {
        self.write_with(|state| {
            let fee_claim_addr = FeeClaimAddress::from_addr(epoch.as_u64(), validator_public_key.as_bytes());
            let claim = state.take_fee_claim(epoch, validator_public_key.clone())?;
            let amount = claim.amount;
            if state.new_fee_claims.insert(fee_claim_addr, claim).is_some() {
                return Err(RuntimeError::DoubleClaimedFee {
                    address: validator_public_key,
                    epoch,
                });
            }
            Ok(ResourceContainer::confidential(
                CONFIDENTIAL_TARI_RESOURCE_ADDRESS,
                None,
                amount,
            ))
        })
    }

    pub fn new_component(
        &self,
        state: Vec<u8>,
        owner_rule: OwnerRule,
        access_rules: ComponentAccessRules,
        component_id: Option<Hash>,
    ) -> Result<ComponentAddress, RuntimeError> {
        let (component_address, component) = self.with_runtime_state(move |runtime_state| {
            let template_address = runtime_state.template_address;
            let component_address = self
                .id_provider()
                .new_component_address(template_address, component_id)?;

            let component = ComponentBody { state };
            let component = ComponentHeader {
                template_address: runtime_state.template_address,
                module_name: runtime_state.template_name.clone(),
                owner_key: runtime_state.transaction_signer_public_key,
                access_rules,
                owner_rule,
                state: component,
            };

            Ok((component_address, component))
        })?;

        let tx_hash = self.transaction_hash();
        self.write_with(|state| {
            // The template address/component_id combination will not necessarily be unique so we need to check this.
            if state.get_component(&component_address).optional()?.is_some() {
                return Err(RuntimeError::ComponentAlreadyExists {
                    address: component_address,
                });
            }

            let template_address = component.template_address;
            let module_name = component.module_name.clone();
            state.new_components.insert(component_address, component);

            state.events.push(Event::new(
                Some(component_address),
                template_address,
                tx_hash,
                "component-created".to_string(),
                Metadata::from([("module_name".to_string(), module_name)]),
            ));

            Ok(())
        })?;

        debug!(target: LOG_TARGET, "New component created: {}", component_address);
        Ok(component_address)
    }

    pub fn get_component(&self, addr: &ComponentAddress) -> Result<ComponentHeader, RuntimeError> {
        self.read_with(|state| state.get_component(addr))
    }

    pub fn component_exists(&self, addr: &ComponentAddress) -> Result<bool, RuntimeError> {
        self.read_with(|state| {
            Ok(state.new_components.contains_key(addr) || state.substate_exists_in_state_store(*addr)?)
        })
    }

    /// Set the component. This may be called many times during execution but always results in exactly one UP substate
    /// with an incremented version.
    pub fn set_component(
        &self,
        component_address: ComponentAddress,
        component: ComponentHeader,
    ) -> Result<(), RuntimeError> {
        self.write_with(|state| {
            state.new_components.insert(component_address, component);
            Ok(())
        })
    }

    pub(crate) fn set_current_runtime_state(&self, state: RuntimeState) {
        self.write_with(|s| s.runtime_state = Some(state));
    }

    pub fn new_vault(
        &self,
        resource_address: ResourceAddress,
        resource_type: ResourceType,
    ) -> Result<VaultId, RuntimeError> {
        let vault_id = self.id_provider.new_vault_id()?;
        debug!(target: LOG_TARGET, "New vault id: {}", vault_id);
        let resource = match resource_type {
            ResourceType::Fungible => ResourceContainer::fungible(resource_address, 0.into()),
            ResourceType::NonFungible => ResourceContainer::non_fungible(resource_address, BTreeSet::new()),
            ResourceType::Confidential => ResourceContainer::confidential(resource_address, None, Amount::zero()),
        };
        let vault = Vault::new(vault_id, resource);

        self.write_with(|state| {
            state.new_vaults.insert(vault_id, vault);
        });

        Ok(vault_id)
    }

    pub fn borrow_vault<R, F: FnOnce(&Vault) -> R>(&self, vault_id: VaultId, f: F) -> Result<R, RuntimeError> {
        self.read_with(|state| state.borrow_vault(vault_id, f))
    }

    pub fn borrow_vault_mut<R, F: FnOnce(&mut Vault) -> R>(&self, vault_id: VaultId, f: F) -> Result<R, RuntimeError> {
        self.write_with(|state| state.borrow_vault_mut(&vault_id, f))
    }

    pub fn vault_exists(&self, vault_id: &VaultId) -> Result<bool, RuntimeError> {
        self.read_with(|state| {
            Ok(state.new_vaults.contains_key(vault_id) || state.substate_exists_in_state_store(*vault_id)?)
        })
    }

    pub fn clone_runtime_state(&self) -> Result<RuntimeState, RuntimeError> {
        self.with_runtime_state(|state| Ok(state.clone()))
    }

    pub fn with_runtime_state<R, F: FnOnce(&RuntimeState) -> Result<R, RuntimeError>>(
        &self,
        f: F,
    ) -> Result<R, RuntimeError> {
        self.read_with(|state| {
            let state = state.runtime_state()?;
            f(state)
        })
    }

    pub fn authorization(&self) -> Authorization<'_> {
        Authorization::new(self)
    }

    pub fn set_last_instruction_output(&self, output: IndexedValue) {
        self.write_with(|state| {
            state.last_instruction_output = Some(output);
        });
    }

    pub fn take_last_instruction_output(&self) -> Option<IndexedValue> {
        self.write_with(|state| state.last_instruction_output.take())
    }

    pub fn get_from_workspace(&self, key: &[u8]) -> Result<IndexedValue, RuntimeError> {
        self.read_with(|state| {
            state
                .workspace
                .get(key)
                .cloned()
                .ok_or(RuntimeError::ItemNotOnWorkspace {
                    key: String::from_utf8_lossy(key).to_string(),
                })
        })
    }

    pub fn with_workspace<F: FnOnce(&Workspace) -> R, R>(&self, f: F) -> R {
        self.read_with(|state| f(&state.workspace))
    }

    pub fn with_workspace_mut<F: FnOnce(&mut Workspace) -> R, R>(&self, f: F) -> R {
        self.write_with(|state| f(&mut state.workspace))
    }

    pub fn pay_fee(&self, resource: ResourceContainer, return_vault: VaultId) -> Result<(), RuntimeError> {
        let mut fee_state = self.fee_state.write().unwrap();
        fee_state.fee_payments.push((resource, return_vault));
        Ok(())
    }

    pub fn add_fee_charge(&self, source: FeeSource, amount: u64) {
        let mut fee_state = self.fee_state.write().unwrap();
        fee_state.fee_charges.push((source, amount));
    }

    pub fn finalize(
        &self,
        mut substates_to_persist: HashMap<SubstateAddress, SubstateValue>,
    ) -> Result<FinalizeTracker, RuntimeError> {
        // Resolve the transfers to the fee pool resource and vault refunds
        let finalized_fee = self.finalize_fees(&mut substates_to_persist)?;

        let transaction_receipt_mut = substates_to_persist
            .get_mut(&SubstateAddress::TransactionReceipt(self.transaction_hash().into()))
            .and_then(|s| s.as_transaction_receipt_mut())
            .expect(
                "InvariantViolation: expected transaction receipt to be added to substates to persist but it wasnt",
            );
        transaction_receipt_mut.fee_receipt = Some(finalized_fee.clone());

        let events = transaction_receipt_mut.events.clone();
        let logs = transaction_receipt_mut.logs.clone();

        // Finalise will always reset the state
        let state = self.take_working_state();

        let result = state
            .validate_finalized()
            .and_then(|_| self.generate_substate_diff(state, substates_to_persist));

        let result = match result {
            Ok(substate_diff) => TransactionResult::Accept(substate_diff),
            Err(err) => TransactionResult::Reject(RejectReason::ExecutionFailure(err.to_string())),
        };

        Ok(FinalizeTracker {
            result,
            events,
            fee_receipt: finalized_fee,
            logs,
        })
    }

    fn generate_substate_diff(
        &self,
        state: WorkingState,
        substates_to_persist: HashMap<SubstateAddress, SubstateValue>,
    ) -> Result<SubstateDiff, TransactionCommitError> {
        let tx = state
            .state_store
            .read_access()
            .map_err(TransactionCommitError::StateStoreTransactionError)?;

        let mut substate_diff = SubstateDiff::new();

        for (address, substate) in substates_to_persist {
            let new_substate = match tx.get_state::<_, Substate>(&address).optional()? {
                Some(existing_state) => {
                    substate_diff.down(address.clone(), existing_state.version());
                    Substate::new(existing_state.version() + 1, substate)
                },
                None => Substate::new(0, substate),
            };
            substate_diff.up(address, new_substate);
        }

        for claimed in state.claimed_confidential_outputs {
            substate_diff.down(SubstateAddress::UnclaimedConfidentialOutput(claimed), 0);
        }

        Ok(substate_diff)
    }

    pub fn fee_checkpoint(&self) -> Result<(), RuntimeError> {
        self.read_with(|state| {
            // Check that the checkpoint is in a valid state
            state.validate_finalized()?;
            let mut checkpoint = self.fee_checkpoint.lock().unwrap();
            *checkpoint = Some(state.clone());
            Ok(())
        })
    }

    pub fn reset_to_fee_checkpoint(&self) -> Result<(), RuntimeError> {
        let mut checkpoint = self.fee_checkpoint.lock().unwrap();
        if let Some(checkpoint) = checkpoint.take() {
            self.write_with(|state| *state = checkpoint);
            Ok(())
        } else {
            Err(RuntimeError::NoCheckpoint)
        }
    }

    fn finalize_fees(
        &self,
        substates_to_persist: &mut HashMap<SubstateAddress, SubstateValue>,
    ) -> Result<FeeReceipt, RuntimeError> {
        let mut fee_state = self.fee_state.write().unwrap();
        let total_fees = fee_state
            .fee_charges
            .iter()
            .map(|(_, fee)| Amount::try_from(*fee).expect("fee overflowed i64::MAX"))
            .sum::<Amount>();
        let total_fee_payment = fee_state
            .fee_payments
            .iter()
            .map(|(resx, _)| resx.amount())
            .sum::<Amount>();

        let mut fee_resource =
            ResourceContainer::confidential(CONFIDENTIAL_TARI_RESOURCE_ADDRESS, None, Amount::zero());

        // Collect the fee
        let mut remaining_fees = total_fees;
        for (resx, _) in &mut fee_state.fee_payments {
            if remaining_fees.is_zero() {
                break;
            }
            let amount_to_withdraw = min(resx.amount(), remaining_fees);
            remaining_fees -= amount_to_withdraw;
            fee_resource.deposit(resx.withdraw(amount_to_withdraw)?)?;
        }

        // Refund the remaining payments if any
        for (mut resx, refund_vault) in fee_state.fee_payments.drain(..) {
            if resx.amount().is_zero() {
                continue;
            }

            let vault = substates_to_persist
                .remove(&refund_vault.into())
                .expect("invariant: vault that made fee payment not in changeset");
            let mut vault = vault.into_vault().unwrap();
            vault.resource_container_mut().deposit(resx.withdraw_all()?)?;
            substates_to_persist.insert(refund_vault.into(), vault.into());
        }

        Ok(FeeReceipt {
            total_fee_payment,
            total_fees_paid: fee_resource.amount(),
            cost_breakdown: fee_state.fee_charges.drain(..).collect(),
        })
    }

    fn take_working_state(&self) -> WorkingState {
        self.write_with(|current_state| {
            mem::replace(
                current_state,
                WorkingState::new(
                    current_state.state_store.clone(),
                    Default::default(),
                    AuthorizationScope::new(vec![]),
                ),
            )
        })
    }

    pub fn take_substates_to_persist(&self) -> HashMap<SubstateAddress, SubstateValue> {
        self.write_with(|state| {
            let total_items = state.new_resources.len() +
                state.new_components.len() +
                state.new_vaults.len() +
                state.new_non_fungibles.len() +
                state.new_non_fungible_indexes.len();
            let mut up_states = HashMap::with_capacity(total_items);

            for (component_addr, substate) in state.new_components.drain() {
                let addr = SubstateAddress::Component(component_addr);
                up_states.insert(addr, substate.into());
            }

            for (vault_id, substate) in state.new_vaults.drain() {
                let addr = SubstateAddress::Vault(vault_id);
                up_states.insert(addr, substate.into());
            }

            for (resource_addr, substate) in state.new_resources.drain() {
                let addr = SubstateAddress::Resource(resource_addr);
                up_states.insert(addr, substate.into());
            }

            for (address, substate) in state.new_non_fungibles.drain() {
                let addr = SubstateAddress::NonFungible(address);
                up_states.insert(addr, substate.into());
            }

            for (address, substate) in state.new_non_fungible_indexes.drain() {
                let addr = SubstateAddress::NonFungibleIndex(address.clone());
                up_states.insert(addr, substate.into());
            }

            for (address, substate) in state.new_fee_claims.drain() {
                let addr = SubstateAddress::FeeClaim(address);
                up_states.insert(addr, substate.into());
            }

            let events = state.events.clone();
            let logs = state.logs.clone();

            let transaction_receipt = TransactionReceipt {
                transaction_hash: self.transaction_hash(),
                events,
                logs,
                fee_receipt: None,
            };

            up_states.insert(
                SubstateAddress::TransactionReceipt(transaction_receipt.transaction_hash.into()),
                SubstateValue::TransactionReceipt(transaction_receipt),
            );

            up_states
        })
    }

    pub fn are_fees_paid_in_full(&self) -> bool {
        let tx = self.fee_state.read().unwrap();
        let total_payments = tx.total_payments();
        let total_charges = Amount::try_from(tx.total_charges()).expect("fee overflowed i64::MAX");
        total_payments >= total_charges
    }

    pub fn total_payments(&self) -> Amount {
        let tx = self.fee_state.read().unwrap();
        tx.total_payments()
    }

    pub fn total_charges(&self) -> Amount {
        let tx = self.fee_state.read().unwrap();
        Amount::try_from(tx.total_charges()).expect("fee overflowed i64::MAX")
    }

    pub(super) fn read_with<R, F: FnOnce(&WorkingState) -> R>(&self, f: F) -> R {
        f(&self.working_state.read().unwrap())
    }

    fn write_with<R, F: FnOnce(&mut WorkingState) -> R>(&self, f: F) -> R {
        f(&mut self.working_state.write().unwrap())
    }

    pub fn transaction_hash(&self) -> Hash {
        self.id_provider.transaction_hash()
    }

    pub(crate) fn id_provider(&self) -> &IdProvider {
        &self.id_provider
    }

    pub fn push_auth_scope(&self) {
        self.write_with(|state| {
            let prev_auth_scope = mem::replace(&mut state.current_auth_scope, AuthorizationScope::new(vec![]));
            state.auth_scope_stack.push(prev_auth_scope);
        });
    }

    pub fn pop_auth_scope(&self) -> Result<(), RuntimeError> {
        self.write_with(|state| {
            let prev_auth_scope = state.auth_scope_stack.pop().ok_or(RuntimeError::AuthScopeStackEmpty)?;
            state.current_auth_scope = prev_auth_scope;
            Ok(())
        })
    }
}
