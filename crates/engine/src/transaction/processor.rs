//  Copyright 2022. The Tari Project
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

use std::{collections::HashSet, sync::Arc, time::Instant};

use log::*;
use ootle_network::Network;
use tari_engine_types::{
    commit_result::{ExecuteResult, FinalizeResult},
    component::derive_component_address_from_public_key,
    entity_id_provider::EntityIdProvider,
    fees::ExhaustBurnRate,
    indexed_value::{IndexedValue, IndexedWellKnownTypes},
    limits,
    lock::LockFlag,
    published_template::TemplateBlob,
    virtual_substate::VirtualSubstates,
};
use tari_ootle_common_types::{optional::Optional, services::template_provider::TemplateProvider};
use tari_ootle_template_metadata::MetadataHash;
use tari_ootle_transaction::{
    AllocatableAddressType,
    ComponentReference,
    Instruction,
    MigrateFunction,
    ResourceAddressRef,
    args::{InstructionArg, WorkspaceId, WorkspaceOffsetId},
    call_arg,
    call_args,
};
use tari_template_abi::FunctionDef;
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::{
    args::{BucketAction, BucketGetAmountArg, BucketRef, WorkspaceAction},
    invoke_args,
    models::{Bucket, BucketId},
    types::{
        Amount,
        NonFungibleAddress,
        OwnerRule,
        TemplateAddress,
        access_rules::ComponentAccessRules,
        constants::STEALTH_TARI_RESOURCE_ADDRESS,
        crypto::RistrettoPublicKeyBytes,
        stealth::StealthTransferStatement,
    },
};

use crate::{
    executables::{Executable, WeightedExecutable},
    fees::WasmMeteringRate,
    runtime::{
        AuthParams,
        AuthorizationScope,
        LoadedComponent,
        NativeAction,
        PayFee,
        Runtime,
        RuntimeError,
        RuntimeInterface,
        RuntimeInterfaceImpl,
        RuntimeModule,
        StateTracker,
        scope::{CallScope, FrameWriteMode, PushCallFrame},
    },
    state_store::StateReader,
    template::LoadedTemplate,
    traits::ClaimProofVerifier,
    transaction::{TransactionError, error::TransactionErrorKind},
    wasm::{Checkout, LoadedWasmTemplate, WasmInstance, WasmModule, WasmProcess},
};

const LOG_TARGET: &str = "tari::ootle::engine::instruction_processor";
const ACCOUNT_CONSTRUCTOR_FUNCTION: &str = "create";
const ACCOUNT_DEPOSIT_METHOD: &str = "deposit";

pub type ModulesCollection<TStore> = Arc<[Box<dyn RuntimeModule<TStore>>]>;

pub struct TransactionProcessor<TStore, TTemplateProvider> {
    template_provider: Arc<TTemplateProvider>,
    state_db: TStore,
    auth_params: AuthParams,
    virtual_substates: VirtualSubstates,
    modules: ModulesCollection<TStore>,
    claim_burn_proof_verifier: Arc<dyn ClaimProofVerifier + Send + Sync + 'static>,
    wasm_metering_rate: WasmMeteringRate,
    /// The exhaust burn rate resolved for the execution epoch and seeded onto the fee state.
    burn_rate: ExhaustBurnRate,
    /// Selects the substate schema version for the execution epoch, which is scheduled per network.
    network: Network,
    dry_run: bool,
}

impl<TStore, TTemplateProvider> TransactionProcessor<TStore, TTemplateProvider>
where
    TStore: StateReader + Clone + 'static,
    TTemplateProvider: TemplateProvider<Template = LoadedTemplate>,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        template_provider: Arc<TTemplateProvider>,
        state_db: TStore,
        auth_params: AuthParams,
        virtual_substates: VirtualSubstates,
        modules: ModulesCollection<TStore>,
        claim_burn_proof_verifier: Arc<dyn ClaimProofVerifier + Send + Sync + 'static>,
        wasm_metering_rate: WasmMeteringRate,
        burn_rate: ExhaustBurnRate,
        network: Network,
        dry_run: bool,
    ) -> Self {
        Self {
            template_provider,
            state_db,
            auth_params,
            virtual_substates,
            modules,
            claim_burn_proof_verifier,
            wasm_metering_rate,
            burn_rate,
            network,
            dry_run,
        }
    }

    #[expect(clippy::too_many_lines)]
    pub fn execute<E: Executable + WeightedExecutable>(self, executable: E) -> Result<ExecuteResult, TransactionError> {
        let (id, intent_commitment) = executable.to_id_and_intent_commitment();
        let timer = Instant::now();
        let entity_id_provider = EntityIdProvider::new(id.as_hash(), 1000);
        let Self {
            template_provider,
            state_db,
            auth_params,
            virtual_substates,
            modules,
            claim_burn_proof_verifier,
            wasm_metering_rate,
            burn_rate,
            network,
            dry_run,
        } = self;

        let execute_epoch = virtual_substates.current_epoch();

        let initial_auth_scope = AuthorizationScope::new(auth_params.initial_ownership_proofs);
        let mut initial_call_scope = CallScope::new();
        initial_call_scope.set_auth_scope(initial_auth_scope);
        // Because XTR resource is immutable, we can make it available to every shard group (genesis state) and
        // transaction (payment of fees)
        initial_call_scope.add_substate_to_owned(STEALTH_TARI_RESOURCE_ADDRESS.into());
        let mut read_declared_inputs = HashSet::new();
        let mut write_declared_inputs = HashSet::new();
        for input in executable.all_inputs_iter() {
            debug!(
                target: LOG_TARGET,
                "Adding substate to initial call scope: {}",
                input
            );
            // Consensus read-locks a read-only substate whatever was declared, so the engine holds it to a read.
            if input.is_write() && !input.substate_id().is_read_only() {
                write_declared_inputs.insert(input.substate_id().clone());
            } else {
                read_declared_inputs.insert(input.substate_id().clone());
            }
            initial_call_scope.add_substate_to_owned(input.into_substate_id());
        }
        // A substate declared both ways is a write: the same rule `declare_input` applies when
        // building the declaration set, repeated here because an `Executable` may chain several.
        read_declared_inputs.retain(|id| !write_declared_inputs.contains(id));

        let transaction_weight = executable.calculate_weight();
        let tracker = StateTracker::new(
            state_db,
            virtual_substates,
            initial_call_scope,
            read_declared_inputs,
            id.as_hash(),
            intent_commitment,
            transaction_weight,
            wasm_metering_rate,
            burn_rate,
            network,
            dry_run,
        );

        if executable.signers_iter().next().is_none() {
            return Err(TransactionErrorKind::InvariantError {
                details: "Transaction must have at least one authorized signature".to_string(),
            }
            .into());
        }
        let main_signer = executable.main_signer();

        let instructions = executable.into_instructions();

        let blobs = std::rc::Rc::new(instructions.blobs);

        let runtime_interface = std::rc::Rc::new(RuntimeInterfaceImpl::initialize(
            tracker,
            template_provider.clone(),
            main_signer,
            entity_id_provider,
            modules,
            claim_burn_proof_verifier,
            std::rc::Rc::clone(&blobs),
        )?) as std::rc::Rc<dyn RuntimeInterface>;

        let runtime = Runtime::new(runtime_interface);

        let transaction_hash = id.as_hash();

        let (runtime, fee_exec_result) =
            Self::process_instructions(&template_provider, runtime, instructions.fee, &blobs);

        match fee_exec_result {
            Ok(()) => {
                // Checkpoint the tracker state after the fee instructions have been executed in case of transaction
                // failure.
                if let Err(err) = runtime.interface().checkpoint_fee_intent() {
                    let mut finalize = FinalizeResult::new_rejected(transaction_hash, err.to_reject_reason(None));
                    // Nothing is taken and nothing is written, but what committing would have cost
                    // is the number the payer has to raise their fee to.
                    let required = runtime.interface().required_fee_payment();
                    finalize.fee_receipt = runtime.interface().metered_fee_receipt();
                    finalize = finalize.with_total_fees_required(required);
                    return Ok(ExecuteResult {
                        finalize,
                        execution_time: timer.elapsed(),
                        execute_epoch: execute_epoch.map(Into::into),
                        wasm_execution_points: runtime.interface().wasm_points_consumed(),
                        native_execution_points: runtime.interface().native_points_consumed(),
                    });
                }
            },
            Err(err) if err.is_node_fault() => return Err(err),
            Err(err) => {
                warn!(
                    target: LOG_TARGET,
                    "Fee payment failed for transaction {}: {}",
                    transaction_hash,
                    err
                );
                let required = runtime.interface().required_fee_payment();
                let mut finalize = FinalizeResult::new_rejected(transaction_hash, err.to_reject_reason());
                finalize.fee_receipt = runtime.interface().metered_fee_receipt();
                return Ok(ExecuteResult {
                    finalize: finalize.with_total_fees_required(required),
                    execution_time: timer.elapsed(),
                    execute_epoch: execute_epoch.map(Into::into),
                    wasm_execution_points: runtime.interface().wasm_points_consumed(),
                    native_execution_points: runtime.interface().native_points_consumed(),
                });
            },
        };

        // Clear the workspace before executing the main instructions
        runtime
            .interface()
            .workspace_invoke(WorkspaceAction::DropAll, invoke_args![].into())?;

        let (runtime, instruction_result) =
            Self::process_instructions(&template_provider, runtime, instructions.main, &blobs);

        // Must be captured before finalize consumes the working state.
        let wasm_execution_points = runtime.interface().wasm_points_consumed();
        let native_execution_points = runtime.interface().native_points_consumed();

        match instruction_result {
            Err(err) if err.is_node_fault() => Err(err),
            Ok(()) => {
                let finalize = runtime.interface().finalize()?;
                Ok(ExecuteResult {
                    finalize,
                    execution_time: timer.elapsed(),
                    execute_epoch: execute_epoch.map(Into::into),
                    wasm_execution_points,
                    native_execution_points,
                })
            },
            // This can happen e.g if you have dangling buckets after running the instructions
            Err(err) => {
                // Reset the state to when the state at the end of the fee instructions. The fee charges for the
                // successful instructions are still charged even though the transaction failed.
                // Finalize will now contain the fee payments and vault refunds only
                let finalize = runtime.interface().finalize_failure(err.to_reject_reason())?;
                Ok(ExecuteResult {
                    finalize,
                    execution_time: timer.elapsed(),
                    execute_epoch: execute_epoch.map(Into::into),
                    wasm_execution_points,
                    native_execution_points,
                })
            },
        }
    }

    fn process_instructions(
        template_provider: &TTemplateProvider,
        runtime: Runtime,
        instructions: Vec<Instruction>,
        blobs: &tari_ootle_transaction::Blobs,
    ) -> (Runtime, Result<(), TransactionError>) {
        let result = instructions
            .into_iter()
            .enumerate()
            .try_for_each(|(idx, instruction)| {
                Self::process_instruction(template_provider, &runtime, instruction, blobs)
                    .and_then(|output| Ok(runtime.interface().complete_instruction(output)?))
                    .map_err(|e| TransactionError::new(idx + 1, e))
            })
            .and_then(|()| {
                // check that the finalized state is valid
                runtime.interface().validate_finalized()?;
                Ok(())
            });

        (runtime, result)
    }

    #[allow(clippy::too_many_lines)]
    fn process_instruction(
        template_provider: &TTemplateProvider,
        runtime: &Runtime,
        instruction: Instruction,
        blobs: &tari_ootle_transaction::Blobs,
    ) -> Result<Option<IndexedValue>, TransactionErrorKind> {
        debug!(target: LOG_TARGET, "instruction = {:?}", instruction);
        match instruction {
            Instruction::CreateAccount {
                owner_public_key: public_key_address,
                owner_rule,
                access_rules,
                bucket_workspace_id: workspace_id,
            } => Self::create_account(
                template_provider,
                runtime,
                &public_key_address,
                owner_rule,
                access_rules,
                workspace_id,
            ),
            Instruction::CallFunction {
                address: template_address,
                function,
                args,
            } => Self::call_function(template_provider, runtime, &template_address, &function, args, None).map(Some),
            Instruction::CallMethod { call, method, args } => {
                Self::call_method(template_provider, runtime, call, &method, args, None).map(Some)
            },
            // Basically names an output on the workspace so that you can refer to it as an
            // Arg::Variable
            Instruction::PutLastInstructionOutputOnWorkspace { key } => {
                Self::put_output_on_workspace_with_id(runtime, key)?;
                Ok(None)
            },
            Instruction::DropAllProofsInWorkspace => {
                Self::drop_all_proofs_in_workspace(runtime)?;
                Ok(None)
            },
            Instruction::ClaimBurn { claim, output_data } => {
                runtime.interface().claim_burn(*claim, output_data)?;
                Ok(None)
            },
            Instruction::ClaimValidatorFees { address, max_amount } => {
                let bucket_id = runtime.interface().claim_validator_fees(address, max_amount)?;
                Ok(Some(IndexedValue::from_type(&bucket_id)?))
            },
            Instruction::Assert { key, assertion } => {
                runtime
                    .interface()
                    .workspace_invoke(WorkspaceAction::Assert, invoke_args![key, assertion].into())?;
                Ok(None)
            },
            Instruction::TakeFromBucket {
                input_bucket,
                amount,
                output_bucket,
            } => {
                let runtime_mut = runtime.interface();
                let item = runtime_mut.workspace_invoke(WorkspaceAction::Get, invoke_args![input_bucket].into())?;

                let bucket_ref = BucketRef::Ref(item.decode()?);
                let bucket = runtime_mut.bucket_invoke(bucket_ref, BucketAction::Take, invoke_args![amount].into())?;
                let prev_bucket_val = runtime_mut
                    .bucket_invoke(
                        bucket_ref,
                        BucketAction::GetAmount,
                        invoke_args![BucketGetAmountArg::Everything].into(),
                    )?
                    .decode::<Amount>()?;
                if prev_bucket_val.is_zero() {
                    // Drop the bucket to prevent a dangling (empty) bucket
                    runtime_mut.bucket_invoke(bucket_ref, BucketAction::DropEmpty, invoke_args![].into())?;
                }

                runtime_mut.put_on_workspace(output_bucket, IndexedValue::from_value(bucket.into_value()?)?)?;
                Ok(None)
            },
            Instruction::PutIntoBucket { src, dest } => {
                let runtime_mut = runtime.interface();
                let src_item = runtime_mut.workspace_invoke(WorkspaceAction::Get, invoke_args![src].into())?;
                let src_bucket_id: BucketId = src_item.decode()?;
                let dest_item = runtime_mut.workspace_invoke(WorkspaceAction::Get, invoke_args![dest].into())?;
                let dest_bucket_ref = BucketRef::Ref(dest_item.decode()?);
                runtime_mut.bucket_invoke(dest_bucket_ref, BucketAction::Join, invoke_args![src_bucket_id].into())?;
                Ok(None)
            },
            Instruction::PublishTemplate { binary, metadata_hash } => {
                let bytes = blobs.get(binary).ok_or(TransactionErrorKind::BlobIndexOutOfBounds {
                    index: binary,
                    count: blobs.len(),
                })?;
                Self::publish_template(runtime, bytes, metadata_hash)
            },
            Instruction::AllocateAddress {
                allocatable_type: substate_type,
                workspace_id,
            } => Self::allocate_address(runtime, substate_type, workspace_id),
            Instruction::StealthTransfer {
                resource_address_ref: resource_address,
                statement,
                revealed_input_bucket,
            } => Self::stealth_transfer(runtime, resource_address, statement, revealed_input_bucket),
            Instruction::PayFeeFromBucket { bucket } => Self::pay_fee_from_bucket(runtime, bucket),
            Instruction::UpdateComponentTemplate {
                component,
                migrate,
                new_template,
            } => Self::update_component_template(template_provider, runtime, component, new_template, migrate),
        }
    }

    fn update_component_template(
        template_provider: &TTemplateProvider,
        runtime: &Runtime,
        component: ComponentReference,
        new_template_addr: TemplateAddress,
        migrate: Option<MigrateFunction>,
    ) -> Result<Option<IndexedValue>, TransactionErrorKind> {
        let component = runtime.interface().load_component(component)?;
        let component_address = component.address;

        let template = template_provider
            .get_template(&new_template_addr)
            .map_err(|e| TransactionErrorKind::FailedToLoadTemplate {
                address: new_template_addr,
                details: e.to_string(),
            })?
            .ok_or(TransactionErrorKind::TemplateNotFound {
                address: new_template_addr,
            })?;

        runtime
            .interface()
            .track_template_loaded(&new_template_addr, template.code_size())?;

        let component_lock = runtime.interface().lock_component(component_address, LockFlag::Write)?;

        let migration_function = migrate
            .as_ref()
            .map(|f| {
                template
                    .template_def()
                    .get_function(&f.name)
                    .cloned()
                    .ok_or_else(|| TransactionErrorKind::FunctionNotFound {
                        name: f.name.to_string(),
                    })
                    .and_then(|f| {
                        if f.is_migration {
                            Ok(f)
                        } else {
                            Err(TransactionErrorKind::NotAMigrationFunction { name: f.name })
                        }
                    })
            })
            .transpose()?;

        let component_scope = component.state_scope;

        let (call_frame, final_args) = if migration_function.is_some() {
            let resolved_args = migrate
                .as_ref()
                .map(|f| {
                    let component_arg = InstructionArg::from_type(&component_address).map_err(|e| {
                        // This should never happen
                        RuntimeError::InvariantError {
                            function: "update_component_template",
                            details: format!("Failed to create component arg: {}", e),
                        }
                    })?;
                    runtime.interface().resolve_args(Some(component_arg), &f.args)
                })
                .transpose()?
                .unwrap_or_default();

            let arg_scope = resolved_args
                .iter()
                .map(IndexedWellKnownTypes::from_value)
                .collect::<Result<_, _>>()?;

            let frame = PushCallFrame::MigrationContext {
                template_address: new_template_addr,
                module_name: template.template_name().to_string(),
                component_scope,
                component_lock,
                arg_scope: Box::new(arg_scope),
                entity_id: component.entity_id,
            };
            (frame, resolved_args)
        } else {
            let frame = PushCallFrame::MigrationContext {
                template_address: new_template_addr,
                module_name: template.template_name().to_string(),
                component_scope,
                component_lock,
                arg_scope: Box::new(IndexedWellKnownTypes::new()),
                entity_id: component.entity_id,
            };
            (frame, vec![])
        };

        // The migration frame runs with the *target* template as its `current_template`, while the
        // component being migrated is the caller's. Only the target template's own migration function
        // executes in this frame, so the template identity is honest — but `template(...)` (and
        // `direct_caller_template(...)`) would become forgeable if any other code ever ran here.
        runtime.interface().push_call_frame(call_frame, None)?;
        // This must come after the call frame as that defines the authorization scope
        runtime
            .interface()
            .check_component_ownership(NativeAction::UpdateComponentTemplate.into())?;
        // A migration with no `migrate` function never reaches `invoke_template`, so this frame's call boundary
        // ends here.
        runtime.interface().revoke_boundary_proofs()?;

        runtime.interface().update_component_template(new_template_addr)?;

        let returned = migration_function
            .map(|function_def| {
                let result =
                    Self::invoke_template(new_template_addr, template, runtime.clone(), &function_def, &final_args)?;
                runtime.interface().validate_return_value(&result)?;
                Ok::<_, TransactionErrorKind>(result)
            })
            .transpose()?;

        let no_return = IndexedWellKnownTypes::new();
        runtime
            .interface()
            .pop_call_frame(returned.as_ref().map_or(&no_return, IndexedValue::well_known_types))?;

        Ok(returned)
    }

    fn pay_fee_from_bucket(
        runtime: &Runtime,
        bucket: WorkspaceOffsetId,
    ) -> Result<Option<IndexedValue>, TransactionErrorKind> {
        runtime.interface().pay_fee(PayFee::FromBucket { bucket })?;
        Ok(None)
    }

    fn stealth_transfer(
        runtime: &Runtime,
        resource_address: ResourceAddressRef,
        statement: StealthTransferStatement,
        revealed_funds_bucket: Option<WorkspaceOffsetId>,
    ) -> Result<Option<IndexedValue>, TransactionErrorKind> {
        let revealed_funds_bucket = revealed_funds_bucket
            .map(|id| {
                runtime.interface().resolve_workspace_id(&id).and_then(|r| {
                    tari_bor::from_value(&r).map_err(|e| RuntimeError::InvalidArgument {
                        argument: "revealed_funds_bucket",
                        reason: format!("Expected workspace id {id} to be a BucketId: {e}"),
                    })
                })
            })
            .transpose()?;
        let maybe_bucket = runtime
            .interface()
            .stealth_transfer(resource_address, statement, revealed_funds_bucket)?;
        Ok(Some(IndexedValue::from_type(&maybe_bucket.map(Bucket::from_id))?))
    }

    fn put_output_on_workspace_with_id(runtime: &Runtime, key: WorkspaceId) -> Result<(), TransactionErrorKind> {
        runtime
            .interface()
            .workspace_invoke(WorkspaceAction::PutLastInstructionOutput, invoke_args![key].into())?;
        Ok(())
    }

    fn drop_all_proofs_in_workspace(runtime: &Runtime) -> Result<(), TransactionErrorKind> {
        runtime
            .interface()
            .workspace_invoke(WorkspaceAction::DropAllProofs, invoke_args![].into())?;
        Ok(())
    }

    /// Allocating a new address for the given [`AllocatableAddressType`].
    fn allocate_address(
        runtime: &Runtime,
        substate_type: AllocatableAddressType,
        workspace_id: WorkspaceId,
    ) -> Result<Option<IndexedValue>, TransactionErrorKind> {
        let entity_id = runtime.interface().next_entity_id()?;
        runtime
            .interface()
            .allocate_address(substate_type, entity_id, workspace_id)?;
        Ok(None)
    }

    /// Load, validate template binary and adds it to TemplateProvider.
    /// Adds a template artifact if successful
    fn publish_template(
        runtime: &Runtime,
        binary: &[u8],
        metadata_hash: Option<MetadataHash>,
    ) -> Result<Option<IndexedValue>, TransactionErrorKind> {
        if binary.len() > limits::ENGINE_LIMITS.max_template_binary_size_bytes {
            return Err(TransactionErrorKind::WasmBinaryTooBig {
                size: binary.len(),
                max: limits::ENGINE_LIMITS.max_template_binary_size_bytes,
            });
        }

        // Every admission rule the module bytes alone can answer runs first, so a module refused by
        // one of them is not billed for a compile nothing performed.
        let structure = WasmModule::prevalidate_code(binary)?;

        // The compile is the most expensive thing a single instruction can ask of a validator, so it
        // is paid for before it runs. The size cap above and the function and variable caps in the
        // prevalidation are what keep this charge affordable.
        runtime
            .interface()
            .charge_template_compile(binary.len() as u64, &structure.compile)?;

        let template_def = WasmModule::compile_prevalidated(binary, structure.shape)?;
        // The size cap above holds the binary within `MAX_TEMPLATE_BLOB_WIRE_BYTES` — a const assertion keeps the two
        // ordered — so constructing TemplateBlob is infallible.
        let blob = TemplateBlob::new_checked(binary).expect("template binary size verified above");
        runtime
            .interface()
            .publish_template(blob, metadata_hash, template_def)?;

        Ok(None)
    }

    #[allow(clippy::too_many_lines)]
    fn create_account(
        template_provider: &TTemplateProvider,
        runtime: &Runtime,
        public_key_address: &RistrettoPublicKeyBytes,
        owner_rule: Option<OwnerRule>,
        access_rules: Option<ComponentAccessRules>,
        workspace_id: Option<WorkspaceOffsetId>,
    ) -> Result<Option<IndexedValue>, TransactionErrorKind> {
        let template = template_provider
            .get_template(&ACCOUNT_TEMPLATE_ADDRESS)
            .map_err(|e| TransactionErrorKind::FailedToLoadTemplate {
                address: ACCOUNT_TEMPLATE_ADDRESS,
                details: e.to_string(),
            })?
            .ok_or(TransactionErrorKind::TemplateNotFound {
                address: ACCOUNT_TEMPLATE_ADDRESS,
            })?;

        // The derived address belongs to `public_key_address`, so anything that departs from the rules that key
        // would get requires that key's signature. Creating the account on the default rules stays permissionless
        // so that a sender can deposit to an account that does not exist yet.
        if owner_rule.is_some() || access_rules.is_some() {
            runtime.interface().check_signer_badge_in_scope(*public_key_address)?;
        }

        let account_address = derive_component_address_from_public_key(&ACCOUNT_TEMPLATE_ADDRESS, public_key_address);

        let maybe_existing_account = runtime
            .interface()
            .load_component(ComponentReference::Address(account_address))
            .optional()?;

        match maybe_existing_account {
            Some(component) => {
                // Component exists, we'll attempt deposit funds into it as necessary

                // Ensure that the existing component is indeed an Account
                if component.template_address != ACCOUNT_TEMPLATE_ADDRESS {
                    return Err(TransactionErrorKind::InvalidCreateAccount {
                        component_address: account_address,
                        details: format!(
                            "A component already exists at the derived account address {}, but it is not an Account \
                             (template address: {})",
                            account_address, component.template_address
                        ),
                    });
                }

                if owner_rule.is_some() || access_rules.is_some() {
                    return Err(TransactionErrorKind::InvalidCreateAccount {
                        component_address: account_address,
                        details: "Cannot specify owner_rule or access_rules when an account already exists at the \
                                  derived address"
                            .to_string(),
                    });
                }

                if let Some(workspace_id) = workspace_id {
                    let args = call_args![WorkspaceOffset(workspace_id)];
                    Self::invoke_component(template, runtime, component, ACCOUNT_DEPOSIT_METHOD, args, None)?;
                }

                // The instruction output is always the ComponentAddress
                Ok(Some(IndexedValue::from_type(&account_address)?))
            },
            None => {
                let function_def = template
                    .template_def()
                    .get_function(ACCOUNT_CONSTRUCTOR_FUNCTION)
                    .cloned()
                    .ok_or_else(|| TransactionErrorKind::FunctionNotFound {
                        name: ACCOUNT_CONSTRUCTOR_FUNCTION.to_string(),
                    })?;

                let mut args = call_args![
                    // the public key NFT is the first argument of the Account template constructor specifying the
                    // default OwnerRule and the component address
                    NonFungibleAddress::from_public_key(*public_key_address),
                    owner_rule,
                    access_rules
                ];

                // add the optional workspace bucket with the initial funds of the account
                if let Some(workspace_id) = workspace_id {
                    args.push(call_arg![WorkspaceOffset(workspace_id)]);
                } else {
                    let none: Option<()> = None;
                    args.push(call_arg![Literal(none)]);
                }

                let resolved_args = runtime.interface().resolve_args(None, &args)?;
                let arg_scope = resolved_args
                    .iter()
                    .map(IndexedWellKnownTypes::from_value)
                    .collect::<Result<_, _>>()?;

                runtime.interface().push_call_frame(
                    PushCallFrame::Static {
                        template_address: ACCOUNT_TEMPLATE_ADDRESS,
                        module_name: template.template_name().to_string(),
                        arg_scope,
                        entity_id: account_address.entity_id(),
                    },
                    None,
                )?;

                let result = Self::invoke_template(
                    ACCOUNT_TEMPLATE_ADDRESS,
                    template,
                    runtime.clone(),
                    &function_def,
                    &resolved_args,
                )?;

                runtime.interface().validate_return_value(&result)?;
                runtime.interface().pop_call_frame(result.well_known_types())?;

                Ok(Some(IndexedValue::from_type(&account_address)?))
            },
        }
    }

    pub(crate) fn call_function(
        template_provider: &TTemplateProvider,
        runtime: &Runtime,
        template_address: &TemplateAddress,
        function: &str,
        args: Vec<InstructionArg>,
        restrict_frame_to: Option<FrameWriteMode>,
    ) -> Result<IndexedValue, TransactionErrorKind> {
        // An account lives at an address derived from its public key, so which rules a component may be created
        // there under is that key's decision. `CreateAccount` is the sole route to the constructor and is where
        // that decision is enforced.
        if *template_address == ACCOUNT_TEMPLATE_ADDRESS && function == ACCOUNT_CONSTRUCTOR_FUNCTION {
            return Err(TransactionErrorKind::CannotCallAccountConstructor);
        }

        let template = template_provider
            .get_template(template_address)
            .map_err(|e| TransactionErrorKind::FailedToLoadTemplate {
                address: *template_address,
                details: e.to_string(),
            })?
            .ok_or_else(|| TransactionErrorKind::TemplateNotFound {
                address: *template_address,
            })?;

        runtime
            .interface()
            .track_template_loaded(template_address, template.code_size())?;

        let function_def = template.template_def().get_function(function).cloned().ok_or_else(|| {
            TransactionErrorKind::FunctionNotFound {
                name: function.to_string(),
            }
        })?;

        if function_def.is_migration {
            return Err(TransactionErrorKind::CannotCallMigrationFunctionDirectly {
                name: function_def.name,
            });
        }

        let resolved_args = runtime.interface().resolve_args(None, &args)?;
        let arg_scope = resolved_args
            .iter()
            .map(IndexedWellKnownTypes::from_value)
            .collect::<Result<_, _>>()?;
        let frame = PushCallFrame::Static {
            template_address: *template_address,
            module_name: template.template_name().to_string(),
            arg_scope,
            entity_id: runtime.interface().next_entity_id()?,
        };

        runtime.interface().push_call_frame(frame, restrict_frame_to)?;

        let result = Self::invoke_template(
            *template_address,
            template,
            runtime.clone(),
            &function_def,
            &resolved_args,
        )?;

        runtime.interface().validate_return_value(&result)?;
        runtime.interface().pop_call_frame(result.well_known_types())?;

        Ok(result)
    }

    pub(crate) fn call_method(
        template_provider: &TTemplateProvider,
        runtime: &Runtime,
        call: ComponentReference,
        method: &str,
        args: Vec<InstructionArg>,
        restrict_frame_to: Option<FrameWriteMode>,
    ) -> Result<IndexedValue, TransactionErrorKind> {
        let component = runtime.interface().load_component(call)?;
        let template_address = component.template_address;

        let template = template_provider
            .get_template(&template_address)
            .map_err(|e| TransactionErrorKind::FailedToLoadTemplate {
                address: template_address,
                details: e.to_string(),
            })?
            .ok_or(TransactionErrorKind::TemplateNotFound {
                address: template_address,
            })?;

        runtime
            .interface()
            .track_template_loaded(&template_address, template.code_size())?;

        Self::invoke_component(template, runtime, component, method, args, restrict_frame_to)
    }

    fn invoke_component(
        template: LoadedTemplate,
        runtime: &Runtime,
        component: LoadedComponent,
        method: &str,
        args: Vec<InstructionArg>,
        restrict_frame_to: Option<FrameWriteMode>,
    ) -> Result<IndexedValue, TransactionErrorKind> {
        let function_def = template.template_def().get_function(method).cloned().ok_or_else(|| {
            TransactionErrorKind::FunctionNotFound {
                name: method.to_string(),
            }
        })?;

        if function_def.is_migration {
            return Err(TransactionErrorKind::CannotCallMigrationFunctionDirectly {
                name: function_def.name,
            });
        }

        let lock_flag = if function_def.is_mut {
            LockFlag::Write
        } else {
            LockFlag::Read
        };

        let component_lock = runtime.interface().lock_component(component.address, lock_flag)?;

        let resolved_args = runtime
            .interface()
            .resolve_args(Some(InstructionArg::from_type(&component.address)?), &args)?;
        let arg_scope = resolved_args
            .iter()
            .skip(1)
            .map(IndexedWellKnownTypes::from_value)
            .collect::<Result<_, _>>()?;

        runtime.interface().push_call_frame(
            PushCallFrame::ForComponent {
                template_address: component.template_address,
                module_name: template.template_name().to_string(),
                component_scope: component.state_scope,
                component_lock,
                arg_scope: Box::new(arg_scope),
                entity_id: component.entity_id,
            },
            restrict_frame_to,
        )?;

        // This must come after the call frame as that defines the authorization scope
        runtime.interface().check_component_access_rules(method)?;

        let result = Self::invoke_template(
            component.template_address,
            template,
            runtime.clone(),
            &function_def,
            &resolved_args,
        )?;

        runtime.interface().validate_return_value(&result)?;
        runtime.interface().pop_call_frame(result.well_known_types())?;
        Ok(result)
    }

    /// Runs a template function in the frame the caller has already pushed. This is the only path from the engine
    /// into template code, so it is where a frame stops holding the proofs that were in scope for its call
    /// boundary: whatever the frame's access rule was evaluated against, the code itself acts with its own badges
    /// and its `Proof` arguments.
    ///
    /// The call runs on the instance of `template_address` the transaction's earlier calls used, if any (see
    /// [`WasmInstanceCache`]), restored first to the state of a fresh instantiation.
    fn invoke_template(
        template_address: TemplateAddress,
        module: LoadedTemplate,
        runtime: Runtime,
        function_def: &FunctionDef,
        args: &[tari_bor::Value],
    ) -> Result<IndexedValue, TransactionErrorKind> {
        runtime.interface().revoke_boundary_proofs()?;

        let result = match module {
            LoadedTemplate::Wasm(loaded) => {
                // Every call pays for an instantiation, whether it gets a new instance or a reused one reset to
                // its fresh state. The charge also bounds the kernel work each call can cause, such as faulting in
                // the module's minimum memory pages again after a reset. It runs before the first metered operator,
                // so it draws on the same allowance and per-block budget as the call's execution.
                runtime.interface().charge_template_instantiation(&loaded.shape())?;
                let checkout = runtime
                    .interface()
                    .wasm_instances()
                    .borrow_mut()
                    .checkout(&template_address);
                let cacheable = !matches!(checkout, Checkout::Reentrant);
                let instance = match checkout {
                    Checkout::Reuse(mut instance) => match instance.process.reset(&mut instance.store) {
                        Ok(()) => Ok(instance),
                        // A reset that fails is a host fault, not something the transaction did: a fresh
                        // instance stands in, at the charge already paid.
                        Err(err) => {
                            warn!(
                                target: LOG_TARGET,
                                "Could not reset the instance of template {template_address}, instantiating afresh: {err}"
                            );
                            Self::create_instance(loaded)
                        },
                    },
                    Checkout::Vacant | Checkout::Reentrant => Self::create_instance(loaded),
                };
                let outcome = instance.and_then(|mut instance| {
                    let result = instance
                        .process
                        .call(&mut instance.store, runtime.clone(), function_def, args)?;
                    Ok((instance, result))
                });

                // Borrowed again only once the call has returned, since a nested call reaches the cache too.
                let mut instances = runtime.interface().wasm_instances().borrow_mut();
                match outcome {
                    Ok((instance, result)) => {
                        if cacheable {
                            instances.checkin(&template_address, instance);
                        }
                        result
                    },
                    Err(err) => {
                        if cacheable {
                            instances.discard(&template_address);
                        }
                        return Err(err);
                    },
                }
            },
        };
        Ok(result)
    }

    fn create_instance(module: LoadedWasmTemplate) -> Result<WasmInstance, TransactionErrorKind> {
        let mut store = module.create_store();
        let process = WasmProcess::init(&mut store, module)?;
        Ok(WasmInstance { store, process })
    }
}
