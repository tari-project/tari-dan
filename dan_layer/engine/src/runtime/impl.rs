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
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

use log::warn;
use tari_bor::encode;
use tari_common_types::types::PublicKey;
use tari_crypto::{
    range_proof::RangeProofService,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    tari_utilities::ByteArray,
};
use tari_dan_common_types::{services::template_provider::TemplateProvider, Epoch};
use tari_engine_types::{
    base_layer_hashing::ownership_proof_hasher,
    commit_result::FinalizeResult,
    component::ComponentHeader,
    confidential::{get_commitment_factory, get_range_proof_service, ConfidentialClaim, ConfidentialOutput},
    events::Event,
    fees::FeeReceipt,
    logs::LogEntry,
    resource_container::ResourceContainer,
    substate::{SubstateAddress, SubstateValue},
};
use tari_template_abi::TemplateDef;
use tari_template_lib::{
    args::{
        Arg,
        BucketAction,
        BucketRef,
        CallAction,
        CallFunctionArg,
        CallMethodArg,
        CallerContextAction,
        ComponentAction,
        ComponentRef,
        ConfidentialRevealArg,
        ConsensusAction,
        CreateComponentArg,
        CreateResourceArg,
        GenerateRandomAction,
        InvokeResult,
        LogLevel,
        MintResourceArg,
        NonFungibleAction,
        PayFeeArg,
        ResourceAction,
        ResourceGetNonFungibleArg,
        ResourceRef,
        ResourceUpdateNonFungibleDataArg,
        VaultAction,
        VaultWithdrawArg,
        WorkspaceAction,
    },
    auth::AccessRules,
    constants::CONFIDENTIAL_TARI_RESOURCE_ADDRESS,
    crypto::RistrettoPublicKeyBytes,
    models::{Amount, BucketId, ComponentAddress, Metadata, NonFungibleAddress, VaultRef},
};

use super::{tracker::FinalizeTracker, AuthorizationScope, Runtime};
use crate::{
    packager::LoadedTemplate,
    runtime::{
        engine_args::EngineArgs,
        tracker::StateTracker,
        AuthParams,
        RuntimeError,
        RuntimeInterface,
        RuntimeModule,
        RuntimeState,
    },
    transaction::TransactionProcessor,
};

const LOG_TARGET: &str = "tari::dan::engine::runtime::impl";

pub struct RuntimeInterfaceImpl<TTemplateProvider: TemplateProvider<Template = LoadedTemplate>> {
    tracker: StateTracker<TTemplateProvider>,
    _auth_params: AuthParams,
    sender_public_key: RistrettoPublicKey,
    modules: Vec<Arc<dyn RuntimeModule<TTemplateProvider>>>,
}

pub struct StateFinalize {
    pub finalized: FinalizeResult,
    pub fee_receipt: FeeReceipt,
}

impl<TTemplateProvider: TemplateProvider<Template = LoadedTemplate>> RuntimeInterfaceImpl<TTemplateProvider> {
    pub fn initialize(
        tracker: StateTracker<TTemplateProvider>,
        auth_params: AuthParams,
        sender_public_key: RistrettoPublicKey,
        modules: Vec<Arc<dyn RuntimeModule<TTemplateProvider>>>,
    ) -> Result<Self, RuntimeError> {
        let runtime = Self {
            tracker,
            _auth_params: auth_params,
            sender_public_key,
            modules,
        };
        runtime.invoke_modules_on_initialize()?;
        Ok(runtime)
    }

    // TODO: this will be needed when we restrict Resources
    // fn check_access_rules(&self, function: FunctionIdent, access_rules: &AccessRules) -> Result<(), RuntimeError> {
    //     // TODO: In this very basic auth system, you can only call on owned objects (because initial_ownership_proofs
    // is     //       usually set to include the owner token).
    //     let auth_zone = AuthorizationScope::new(&self.auth_params.initial_ownership_proofs);
    //     auth_zone.check_access_rules(&function, access_rules)
    // }

    fn invoke_modules_on_initialize(&self) -> Result<(), RuntimeError> {
        for module in &self.modules {
            module.on_initialize(&self.tracker)?;
        }
        Ok(())
    }

    fn invoke_modules_on_runtime_call(&self, function: &'static str) -> Result<(), RuntimeError> {
        for module in &self.modules {
            module.on_runtime_call(&self.tracker, function)?;
        }
        Ok(())
    }

    fn invoke_modules_on_before_finalize(
        &self,
        substates_to_persist: &HashMap<SubstateAddress, SubstateValue>,
    ) -> Result<(), RuntimeError> {
        for module in &self.modules {
            module.on_before_finalize(&self.tracker, substates_to_persist)?;
        }
        Ok(())
    }
}

impl<TTemplateProvider: TemplateProvider<Template = LoadedTemplate>> RuntimeInterface
    for RuntimeInterfaceImpl<TTemplateProvider>
{
    fn set_current_runtime_state(&self, state: RuntimeState) -> Result<(), RuntimeError> {
        self.invoke_modules_on_runtime_call("set_current_runtime_state")?;
        self.tracker.set_current_runtime_state(state);
        Ok(())
    }

    fn emit_event(&self, topic: String, payload: Metadata) -> Result<(), RuntimeError> {
        self.invoke_modules_on_runtime_call("emit_event")?;

        let component_address = self.tracker.runtime_state_component_address()?;
        let tx_hash = self.tracker.transaction_hash();
        let template_address = self.tracker.get_template_address()?;

        let event = Event::new(component_address, template_address, tx_hash, topic, payload);
        log::log!(target: "tari::dan::engine::runtime", log::Level::Debug, "{}", event.to_string());
        self.tracker.add_event(event);
        Ok(())
    }

    fn emit_log(&self, level: LogLevel, message: String) -> Result<(), RuntimeError> {
        self.invoke_modules_on_runtime_call("emit_log")?;

        let log_level = match level {
            LogLevel::Error => log::Level::Error,
            LogLevel::Warn => log::Level::Warn,
            LogLevel::Info => log::Level::Info,
            LogLevel::Debug => log::Level::Debug,
        };

        // eprintln!("{}: {}", log_level, message);
        log::log!(target: "tari::dan::engine::runtime", log_level, "{}", message);
        self.tracker.add_log(LogEntry::new(level, message));
        Ok(())
    }

    fn get_component(&self, address: &ComponentAddress) -> Result<ComponentHeader, RuntimeError> {
        self.invoke_modules_on_runtime_call("get_component")?;
        self.tracker.get_component(address)
    }

    fn caller_context_invoke(&self, action: CallerContextAction) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("caller_context_invoke")?;

        let sender_public_key = RistrettoPublicKeyBytes::from_bytes(self.sender_public_key.as_bytes()).expect(
            "RistrettoPublicKeyBytes::from_bytes should be infallible when called with RistrettoPublicKey bytes",
        );

        match action {
            CallerContextAction::GetCallerPublicKey => Ok(InvokeResult::encode(&sender_public_key)?),
        }
    }

    fn component_invoke(
        &self,
        component_ref: ComponentRef,
        action: ComponentAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("component_invoke")?;

        match action {
            ComponentAction::Create => {
                let arg: CreateComponentArg = args.get(0)?;
                let template_def = self.tracker.get_template_def()?;
                validate_access_rules(&arg.access_rules, &template_def)?;
                let component_address =
                    self.tracker
                        .new_component(arg.encoded_state, arg.access_rules, arg.component_id)?;
                Ok(InvokeResult::encode(&component_address)?)
            },

            ComponentAction::Get => {
                let address = component_ref
                    .as_component_address()
                    .ok_or_else(|| RuntimeError::InvalidArgument {
                        argument: "component_ref",
                        reason: "Get component action requires a component address".to_string(),
                    })?;
                let component = self.tracker.get_component(&address)?;
                Ok(InvokeResult::encode(&component.state.state)?)
            },
            ComponentAction::SetState => {
                let address = component_ref
                    .as_component_address()
                    .ok_or_else(|| RuntimeError::InvalidArgument {
                        argument: "component_ref",
                        reason: "SetState component action requires a component address".to_string(),
                    })?;
                let state = args.get(0)?;
                let mut component = self.tracker.get_component(&address)?;
                // TODO: Need to validate this state somehow - it could contain arbitrary data incl. vaults that are not
                //       owned by this component.
                component.state.set(state);
                self.tracker.set_component(address, component)?;
                Ok(InvokeResult::unit())
            },
            ComponentAction::SetAccessRules => {
                let address = component_ref
                    .as_component_address()
                    .ok_or_else(|| RuntimeError::InvalidArgument {
                        argument: "component_ref",
                        reason: "SetAccessRules component action requires a component address".to_string(),
                    })?;
                let access_rules: AccessRules = args.get(0)?;
                let mut component = self.tracker.get_component(&address)?;

                component.access_rules = access_rules;
                self.tracker.set_component(address, component)?;
                Ok(InvokeResult::unit())
            },
        }
    }

    fn resource_invoke(
        &self,
        resource_ref: ResourceRef,
        action: ResourceAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("resource_invoke")?;

        match action {
            ResourceAction::Create => {
                let arg: CreateResourceArg = args.get(0)?;

                let resource_address =
                    self.tracker
                        .new_resource(arg.resource_type, arg.token_symbol.clone(), arg.metadata)?;

                let mut output_bucket = None;
                if let Some(mint_arg) = arg.mint_arg {
                    let bucket_id = self.tracker.mint_resource(resource_address, mint_arg)?;
                    output_bucket = Some(tari_template_lib::models::Bucket::from_id(bucket_id));
                }

                Ok(InvokeResult::encode(&(resource_address, output_bucket))?)
            },

            ResourceAction::GetTotalSupply => {
                let resource_address =
                    resource_ref
                        .as_resource_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "resource_ref",
                            reason: "GetResourceType resource action requires a resource address".to_string(),
                        })?;
                let resource = self.tracker.get_resource(&resource_address)?;
                // TODO: access check
                // self.check_access_rules(
                //     FunctionIdent::Native(NativeFunctionCall::Resource(ResourceAction::GetTotalSupply)),
                //     &component.access_rules,
                // )?;
                let total_supply = resource.total_supply();
                Ok(InvokeResult::encode(&total_supply)?)
            },
            ResourceAction::GetResourceType => {
                let resource_address =
                    resource_ref
                        .as_resource_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "resource_ref",
                            reason: "GetResourceType resource action requires a resource address".to_string(),
                        })?;
                let resource = self.tracker.get_resource(&resource_address)?;
                // TODO: access check
                let resource_type = resource.resource_type();
                Ok(InvokeResult::encode(&resource_type)?)
            },
            ResourceAction::Mint => {
                let resource_address =
                    resource_ref
                        .as_resource_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "resource_ref",
                            reason: "Mint resource action requires a resource address".to_string(),
                        })?;
                let mint_resource: MintResourceArg = args.get(0)?;
                // TODO: access check
                let bucket_id = self.tracker.mint_resource(resource_address, mint_resource.mint_arg)?;
                let bucket = tari_template_lib::models::Bucket::from_id(bucket_id);
                Ok(InvokeResult::encode(&bucket)?)
            },
            ResourceAction::GetNonFungible => {
                let resource_address =
                    resource_ref
                        .as_resource_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "resource_ref",
                            reason: "GetNonFungible resource action requires a resource address".to_string(),
                        })?;
                let arg: ResourceGetNonFungibleArg = args.get(0)?;
                let nf_container = self
                    .tracker
                    .get_non_fungible(&NonFungibleAddress::new(resource_address, arg.id.clone()))?;
                // TODO: access check
                if nf_container.is_burnt() {
                    return Err(RuntimeError::InvalidOpNonFungibleBurnt {
                        op: "GetNonFungible",
                        nf_id: arg.id,
                        resource_address,
                    });
                }
                Ok(InvokeResult::encode(&tari_template_lib::models::NonFungible::new(
                    NonFungibleAddress::new(resource_address, arg.id),
                ))?)
            },
            ResourceAction::UpdateNonFungibleData => {
                let resource_address =
                    resource_ref
                        .as_resource_address()
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "resource_ref",
                            reason: "UpdateNonFungibleData resource action requires a resource address".to_string(),
                        })?;
                let arg: ResourceUpdateNonFungibleDataArg = args.get(0)?;
                // TODO: access check
                self.tracker
                    .set_non_fungible_data(&NonFungibleAddress::new(resource_address, arg.id), arg.data)?;

                Ok(InvokeResult::unit())
            },
        }
    }

    #[allow(clippy::too_many_lines)]
    fn vault_invoke(
        &self,
        vault_ref: VaultRef,
        action: VaultAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("vault_invoke")?;

        match action {
            VaultAction::Create => {
                let resource_address = vault_ref
                    .resource_address()
                    .ok_or_else(|| RuntimeError::InvalidArgument {
                        argument: "vault_ref",
                        reason: "Create vault action requires a resource address".to_string(),
                    })?;
                let resource = self.tracker.get_resource(resource_address)?;

                let vault_id = self.tracker.new_vault(*resource_address, resource.resource_type())?;
                Ok(InvokeResult::encode(&vault_id)?)
            },
            VaultAction::Deposit => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "Put vault action requires a vault id".to_string(),
                })?;
                let bucket_id: BucketId = args.get(0)?;
                // TODO: access check

                let bucket = self.tracker.take_bucket(bucket_id)?;
                self.tracker
                    .borrow_vault_mut(vault_id, |vault| vault.deposit(bucket))??;
                Ok(InvokeResult::unit())
            },
            VaultAction::Withdraw => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "WithdrawFungible vault action requires a vault id".to_string(),
                })?;
                let arg: VaultWithdrawArg = args.get(0)?;

                // TODO: access check
                let resource = self.tracker.borrow_vault_mut(vault_id, |vault| match arg {
                    VaultWithdrawArg::Fungible { amount } => vault.withdraw(amount),
                    VaultWithdrawArg::NonFungible { ids } => vault.withdraw_non_fungibles(&ids),
                    VaultWithdrawArg::Confidential { proof } => vault.withdraw_confidential(*proof),
                })??;
                let bucket = self.tracker.new_bucket(resource)?;
                Ok(InvokeResult::encode(&bucket)?)
            },
            VaultAction::WithdrawAll => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "WithdrawAll vault action requires a vault id".to_string(),
                })?;

                // TODO: access check
                let resource = self
                    .tracker
                    .borrow_vault_mut(vault_id, |vault| vault.withdraw_all())??;
                let bucket = self.tracker.new_bucket(resource)?;
                Ok(InvokeResult::encode(&bucket)?)
            },
            VaultAction::GetBalance => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "GetBalance vault action requires a vault id".to_string(),
                })?;
                // TODO: access check

                let balance = self.tracker.borrow_vault(vault_id, |v| v.balance())?;
                Ok(InvokeResult::encode(&balance)?)
            },
            VaultAction::GetResourceAddress => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "vault action requires a vault id".to_string(),
                })?;

                // TODO: access check
                let address = self
                    .tracker
                    .borrow_vault_mut(vault_id, |vault| *vault.resource_address())?;
                Ok(InvokeResult::encode(&address)?)
            },
            VaultAction::GetNonFungibleIds => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "vault action requires a vault id".to_string(),
                })?;

                // TODO: access check
                let resp = self.tracker.borrow_vault(vault_id, |vault| {
                    let empty = BTreeSet::new();
                    let ids = vault.get_non_fungible_ids().unwrap_or(&empty);
                    // NOTE: A BTreeSet does not decode when received in the WASM
                    InvokeResult::encode(&ids.iter().collect::<Vec<_>>())
                })??;

                Ok(resp)
            },
            VaultAction::GetCommitmentCount => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "vault action requires a vault id".to_string(),
                })?;

                self.tracker.borrow_vault(vault_id, |vault| {
                    let count = vault.get_commitment_count();
                    Ok(InvokeResult::encode(&count)?)
                })?
            },
            VaultAction::ConfidentialReveal => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "vault action requires a vault id".to_string(),
                })?;

                let arg: ConfidentialRevealArg = args.get(0)?;

                // TODO: access check
                let resource = self
                    .tracker
                    .borrow_vault_mut(vault_id, |vault| vault.reveal_confidential(arg.proof))??;

                let bucket_id = self.tracker.new_bucket(resource)?;
                Ok(InvokeResult::encode(&bucket_id)?)
            },
            VaultAction::PayFee => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "vault action requires a vault id".to_string(),
                })?;

                let arg: PayFeeArg = args.get(0)?;
                if arg.amount.is_negative() {
                    return Err(RuntimeError::InvalidArgument {
                        argument: "amount",
                        reason: "Amount must be positive".to_string(),
                    });
                }

                let resource = self.tracker.borrow_vault_mut(vault_id, |vault| {
                    let mut resource = ResourceContainer::confidential(*vault.resource_address(), None, Amount::zero());
                    if !arg.amount.is_zero() {
                        let withdrawn = vault.withdraw(arg.amount)?;
                        resource.deposit(withdrawn)?;
                    }
                    if let Some(proof) = arg.proof {
                        let revealed = vault.reveal_confidential(proof)?;
                        resource.deposit(revealed)?;
                    }
                    if resource.amount().is_zero() {
                        return Err(RuntimeError::InvalidArgument {
                            argument: "TakeFeesArg",
                            reason: "Fee payment has zero value".to_string(),
                        });
                    }
                    Ok(resource)
                })??;

                self.tracker.pay_fee(resource, vault_id)?;
                Ok(InvokeResult::unit())
            },
        }
    }

    fn bucket_invoke(
        &self,
        bucket_ref: BucketRef,
        action: BucketAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("bucket_invoke")?;

        match action {
            BucketAction::Create => {
                let resource_address = bucket_ref
                    .resource_address()
                    .ok_or_else(|| RuntimeError::InvalidArgument {
                        argument: "bucket_ref",
                        reason: "Create bucket action requires a resource address".to_string(),
                    })?;
                let resource = self.tracker.get_resource(&resource_address)?;
                let bucket_id = self
                    .tracker
                    .new_empty_bucket(resource_address, resource.resource_type())?;
                Ok(InvokeResult::encode(&bucket_id)?)
            },
            BucketAction::GetResourceAddress => {
                let bucket_id = bucket_ref.bucket_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "bucket_ref",
                    reason: "GetResourceAddress action requires a bucket id".to_string(),
                })?;
                let bucket = self.tracker.get_bucket(bucket_id)?;
                Ok(InvokeResult::encode(bucket.resource_address())?)
            },
            BucketAction::GetResourceType => {
                let bucket_id = bucket_ref.bucket_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "bucket_ref",
                    reason: "GetResourceType action requires a bucket id".to_string(),
                })?;
                let bucket = self.tracker.get_bucket(bucket_id)?;
                Ok(InvokeResult::encode(&bucket.resource_type())?)
            },
            BucketAction::GetAmount => {
                let bucket_id = bucket_ref.bucket_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "bucket_ref",
                    reason: "GetAmount bucket action requires a bucket id".to_string(),
                })?;
                let bucket = self.tracker.get_bucket(bucket_id)?;
                Ok(InvokeResult::encode(&bucket.amount())?)
            },
            BucketAction::Take => {
                let bucket_id = bucket_ref.bucket_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "bucket_ref",
                    reason: "Take bucket action requires a bucket id".to_string(),
                })?;
                let amount = args.get(0)?;
                let resource = self
                    .tracker
                    .with_bucket_mut(bucket_id, |bucket| bucket.take(amount))??;
                let bucket_id = self.tracker.new_bucket(resource)?;
                Ok(InvokeResult::encode(&bucket_id)?)
            },
            BucketAction::TakeConfidential => {
                let bucket_id = bucket_ref.bucket_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "bucket_ref",
                    reason: "Take bucket action requires a bucket id".to_string(),
                })?;
                let proof = args.get(0)?;
                let resource = self
                    .tracker
                    .with_bucket_mut(bucket_id, |bucket| bucket.take_confidential(proof))??;
                let bucket_id = self.tracker.new_bucket(resource)?;
                Ok(InvokeResult::encode(&bucket_id)?)
            },
            BucketAction::RevealConfidential => {
                let bucket_id = bucket_ref.bucket_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "bucket_ref",
                    reason: "RevealConfidential bucket action requires a bucket id".to_string(),
                })?;
                let proof = args.get(0)?;
                let resource = self
                    .tracker
                    .with_bucket_mut(bucket_id, |bucket| bucket.reveal_confidential(proof))??;
                let bucket_id = self.tracker.new_bucket(resource)?;
                Ok(InvokeResult::encode(&bucket_id)?)
            },
            BucketAction::Burn => {
                let bucket_id = bucket_ref.bucket_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "bucket_ref",
                    reason: "Burn bucket action requires a bucket id".to_string(),
                })?;
                self.tracker.burn_bucket(bucket_id)?;
                Ok(InvokeResult::unit())
            },
        }
    }

    fn workspace_invoke(&self, action: WorkspaceAction, args: EngineArgs) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("workspace_invoke")?;
        match action {
            WorkspaceAction::ListBuckets => {
                let bucket_ids = self.tracker.list_buckets();
                Ok(InvokeResult::encode(&bucket_ids)?)
            },
            WorkspaceAction::Put => todo!(),
            // Basically names an output on the workspace so that you can refer to it as an
            // Arg::Variable
            WorkspaceAction::PutLastInstructionOutput => {
                let key = args.get(0)?;
                let last_output = self
                    .tracker
                    .take_last_instruction_output()
                    .ok_or(RuntimeError::NoLastInstructionOutput)?;
                self.tracker.put_in_workspace(key, last_output)?;
                Ok(InvokeResult::unit())
            },
            WorkspaceAction::Get => {
                let key: Vec<u8> = args.get(0)?;
                let value = self.tracker.get_from_workspace(&key)?;
                Ok(InvokeResult::encode(&value)?)
            },
        }
    }

    fn non_fungible_invoke(
        &self,
        nf_addr: NonFungibleAddress,
        action: NonFungibleAction,
        _args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("non_fungible_invoke")?;
        match action {
            NonFungibleAction::GetData => {
                let container = self.tracker.get_non_fungible(&nf_addr)?;
                // TODO: access check
                let contents = container
                    .contents()
                    .ok_or_else(|| RuntimeError::InvalidOpNonFungibleBurnt {
                        op: "GetData",
                        resource_address: *nf_addr.resource_address(),
                        nf_id: nf_addr.id().clone(),
                    })?;

                Ok(InvokeResult::raw(contents.data().to_vec()))
            },
            NonFungibleAction::GetMutableData => {
                let container = self.tracker.get_non_fungible(&nf_addr)?;
                // TODO: access check
                let contents = container
                    .contents()
                    .ok_or_else(|| RuntimeError::InvalidOpNonFungibleBurnt {
                        op: "GetMutableData",
                        resource_address: *nf_addr.resource_address(),
                        nf_id: nf_addr.id().clone(),
                    })?;

                Ok(InvokeResult::raw(contents.mutable_data().to_vec()))
            },
        }
    }

    fn consensus_invoke(&self, action: ConsensusAction) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("consensus_invoke")?;
        match action {
            ConsensusAction::GetCurrentEpoch => {
                let epoch = self.tracker.get_current_epoch()?;
                Ok(InvokeResult::encode(&epoch)?)
            },
        }
    }

    fn generate_random_invoke(&self, action: GenerateRandomAction) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("generate_random_invoke")?;
        match action {
            GenerateRandomAction::GetRandomBytes { len } => {
                let random = self.tracker.id_provider().get_random_bytes(len)?;
                Ok(InvokeResult::encode(&random)?)
            },
        }
    }

    fn call_invoke(&self, action: CallAction, args: EngineArgs) -> Result<InvokeResult, RuntimeError> {
        self.invoke_modules_on_runtime_call("call_invoke")?;

        // we are initializing a new runtime for the nested call
        let call_runtime_interface = RuntimeInterfaceImpl::initialize(
            self.tracker.clone(),
            // for safety reasons we are not going to propagate proofs to the call component or template
            AuthParams {
                initial_ownership_proofs: vec![],
            },
            self.sender_public_key.clone(),
            self.modules.clone(),
        )?;

        // for safety reasons, we explicitly avoid propagating any auth scope to the template/component we are calling
        // so all nested calls can only be done to not auth restricted template methods.
        // this prevents, for example, to withdraw from an account in a nested call
        let auth_scope = AuthorizationScope::new(Arc::new(vec![]));

        // extract all the common required information for both types of calls
        let template_provider = self.tracker.get_template_provider();
        let current_runtime_state = self.tracker.runtime_state()?;
        let max_recursion_depth = current_runtime_state.max_recursion_depth;

        // a nested call starts with an increased recursion depth
        let new_recursion_depth = current_runtime_state.recursion_depth + 1;

        // the runtime state will be overriden by the call so we store a copy to revert back afterwards
        let caller_runtime_state = self.tracker.runtime_state()?;

        let exec_result = match action {
            CallAction::CallFunction => {
                // extract the args from the invoke operation
                let arg: CallFunctionArg = args.get(0)?;
                let template_address = arg.template_address;
                let template_name = template_provider
                    .get_template_module(&template_address)
                    .map_err(|e| RuntimeError::FailedToLoadTemplate {
                        address: template_address,
                        details: e.to_string(),
                    })?
                    .ok_or(RuntimeError::TemplateNotFound { template_address })?
                    .template_name()
                    .to_string();
                let function = arg.function;
                let args = arg.args.into_iter().map(Arg::Literal).collect();

                let new_state = RuntimeState {
                    template_name,
                    template_address,
                    component_address: None,
                    recursion_depth: new_recursion_depth,
                    max_recursion_depth,
                };
                call_runtime_interface.set_current_runtime_state(new_state)?;
                let call_runtime = Runtime::new(Arc::new(call_runtime_interface));

                TransactionProcessor::call_function(
                    template_provider,
                    &call_runtime,
                    auth_scope,
                    &template_address,
                    &function,
                    // TODO: put in rest of args
                    args,
                    new_recursion_depth,
                    max_recursion_depth,
                )
                .map_err(|e| RuntimeError::CallFunctionError {
                    template_address,
                    function,
                    details: e.to_string(),
                })?
            },
            CallAction::CallMethod => {
                // extract the args from the invoke operation
                let arg: CallMethodArg = args.get(0)?;
                let component_address = arg.component_address;
                let component_header = self.tracker.get_component(&component_address)?;
                let template_name = component_header.module_name;
                let template_address = component_header.template_address;
                let method = arg.method;
                let args = arg.args.into_iter().map(Arg::Literal).collect();

                let new_state = RuntimeState {
                    template_name,
                    template_address,
                    component_address: Some(component_address),
                    recursion_depth: new_recursion_depth,
                    max_recursion_depth,
                };
                call_runtime_interface.set_current_runtime_state(new_state)?;
                let call_runtime = Runtime::new(Arc::new(call_runtime_interface));

                TransactionProcessor::call_method(
                    template_provider,
                    &call_runtime,
                    auth_scope,
                    &component_address,
                    &method,
                    args,
                    new_recursion_depth,
                    max_recursion_depth,
                )
                .map_err(|e| RuntimeError::CallMethodError {
                    component_address,
                    method,
                    details: e.to_string(),
                })?
            },
        };

        // the runtime state was overriden by the call so we revert
        self.tracker.set_current_runtime_state(caller_runtime_state);

        Ok(InvokeResult::raw(exec_result.raw))
    }

    fn generate_uuid(&self) -> Result<[u8; 32], RuntimeError> {
        self.invoke_modules_on_runtime_call("generate_uuid")?;
        let uuid = self.tracker.id_provider().new_uuid()?;
        Ok(uuid)
    }

    fn set_last_instruction_output(&self, value: Option<Vec<u8>>) -> Result<(), RuntimeError> {
        self.invoke_modules_on_runtime_call("set_last_instruction_output")?;
        self.tracker.set_last_instruction_output(value);
        Ok(())
    }

    fn claim_burn(&self, claim: ConfidentialClaim) -> Result<(), RuntimeError> {
        let ConfidentialClaim {
            public_key: diffie_hellman_public_key,
            output_address,
            range_proof,
            proof_of_knowledge,
            withdraw_proof,
        } = claim;
        // 1. Must exist
        let unclaimed_output = self.tracker.take_unclaimed_confidential_output(output_address)?;
        // 2. owner_sig must be valid
        let challenge = ownership_proof_hasher()
            .chain_update(proof_of_knowledge.public_nonce())
            .chain_update(&unclaimed_output.commitment)
            .chain_update(&self.sender_public_key)
            .result();

        if !proof_of_knowledge.verify(
            &unclaimed_output.commitment,
            &RistrettoSecretKey::from_bytes(&challenge).map_err(|_e| RuntimeError::InvalidClaimingSignature)?,
            get_commitment_factory(),
        ) {
            warn!(target: LOG_TARGET, "Claim burn failed - Invalid signature");
            return Err(RuntimeError::InvalidClaimingSignature);
        }

        // 3. range_proof must be valid
        if !get_range_proof_service(1).verify(&range_proof, &unclaimed_output.commitment) {
            warn!(target: LOG_TARGET, "Claim burn failed - Invalid range proof");
            return Err(RuntimeError::InvalidRangeProof);
        }

        // 4. Create the confidential resource
        let mut resource = ResourceContainer::confidential(
            *CONFIDENTIAL_TARI_RESOURCE_ADDRESS,
            Some((
                unclaimed_output.commitment.as_public_key().clone(),
                ConfidentialOutput {
                    commitment: unclaimed_output.commitment,
                    stealth_public_nonce: diffie_hellman_public_key,
                    encrypted_data: unclaimed_output.encrypted_data,
                    minimum_value_promise: 0,
                },
            )),
            Amount::zero(),
        );

        // If a withdraw proof is provided, we execute it and deposit back into the resource
        // This allows some funds to be revealed and/or reblinded within a single instruction
        if let Some(proof) = withdraw_proof {
            let withdraw = resource.withdraw_confidential(proof)?;
            resource.deposit(withdraw)?;
        }

        let bucket_id = self.tracker.new_bucket(resource)?;

        self.tracker.set_last_instruction_output(Some(encode(&bucket_id)?));
        Ok(())
    }

    fn claim_validator_fees(&self, epoch: Epoch, validator_public_key: PublicKey) -> Result<(), RuntimeError> {
        let resource = self.tracker.claim_fee(epoch, validator_public_key)?;
        let bucket_id = self.tracker.new_bucket(resource)?;
        self.tracker.set_last_instruction_output(Some(encode(&bucket_id)?));
        Ok(())
    }

    fn create_free_test_coins(
        &self,
        revealed_amount: Amount,
        output: Option<ConfidentialOutput>,
    ) -> Result<BucketId, RuntimeError> {
        let resource = ResourceContainer::confidential(
            *CONFIDENTIAL_TARI_RESOURCE_ADDRESS,
            output.map(|o| (o.commitment.as_public_key().clone(), o)),
            revealed_amount,
        );

        let bucket_id = self.tracker.new_bucket(resource)?;
        self.tracker.set_last_instruction_output(Some(encode(&bucket_id)?));
        Ok(bucket_id)
    }

    fn fee_checkpoint(&self) -> Result<(), RuntimeError> {
        if self.tracker.total_payments() < self.tracker.total_charges() {
            return Err(RuntimeError::InsufficientFeesPaid {
                required_fee: self.tracker.total_charges(),
                fees_paid: self.tracker.total_payments(),
            });
        }
        self.tracker.fee_checkpoint()
    }

    fn reset_to_fee_checkpoint(&self) -> Result<(), RuntimeError> {
        warn!(target: LOG_TARGET, "Resetting to fee checkpoint");
        self.tracker.reset_to_fee_checkpoint()
    }

    fn finalize(&self) -> Result<StateFinalize, RuntimeError> {
        self.invoke_modules_on_runtime_call("finalize")?;

        // TODO: this should not be checked here because it will silently fail
        // and the transaction will think it succeeds. Rather move this check to the transaction
        // processor and reset to fee checkpoint there.
        if !self.tracker.are_fees_paid_in_full() {
            self.reset_to_fee_checkpoint()?;
        }

        let substates_to_persist = self.tracker.take_substates_to_persist();
        self.invoke_modules_on_before_finalize(&substates_to_persist)?;

        let FinalizeTracker {
            result,
            fee_receipt,
            events,
            logs,
        } = self.tracker.finalize(substates_to_persist)?;
        let finalized = FinalizeResult::new(
            self.tracker.transaction_hash(),
            logs,
            events,
            result,
            fee_receipt.to_cost_breakdown(),
        );

        Ok(StateFinalize { finalized, fee_receipt })
    }
}

fn validate_access_rules(access_rules: &AccessRules, template_def: &TemplateDef) -> Result<(), RuntimeError> {
    for (name, _) in access_rules.method_access_rules_iter() {
        if template_def.functions.iter().all(|f| f.name != *name) {
            return Err(RuntimeError::InvalidMethodAccessRule {
                template_name: template_def.template_name.clone(),
                details: format!("No method '{}' found in template", name),
            });
        }
    }
    Ok(())
}
