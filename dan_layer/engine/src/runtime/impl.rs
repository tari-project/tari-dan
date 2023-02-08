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

use std::collections::BTreeSet;

use tari_engine_types::{
    commit_result::{FinalizeResult, RejectReason, TransactionResult},
    logs::LogEntry,
};
use tari_template_abi::TemplateDef;
use tari_template_lib::{
    args::{
        BucketAction,
        BucketRef,
        ComponentAction,
        ComponentRef,
        ConsensusAction,
        CreateComponentArg,
        CreateResourceArg,
        InvokeResult,
        LogLevel,
        MintResourceArg,
        NonFungibleAction,
        ResourceAction,
        ResourceGetNonFungibleArg,
        ResourceRef,
        ResourceUpdateNonFungibleDataArg,
        VaultAction,
        VaultWithdrawArg,
        WorkspaceAction,
    },
    auth::AccessRules,
    models::{BucketId, ComponentAddress, ComponentHeader, NonFungibleAddress, VaultRef},
};

use crate::runtime::{
    engine_args::EngineArgs,
    tracker::StateTracker,
    AuthParams,
    ConsensusContext,
    RuntimeError,
    RuntimeInterface,
    RuntimeModule,
    RuntimeState,
};

pub struct RuntimeInterfaceImpl {
    tracker: StateTracker,
    _auth_params: AuthParams,
    consensus: ConsensusContext,
    modules: Vec<Box<dyn RuntimeModule>>,
}

impl RuntimeInterfaceImpl {
    pub fn new(
        tracker: StateTracker,
        auth_params: AuthParams,
        consensus: ConsensusContext,
        modules: Vec<Box<dyn RuntimeModule>>,
    ) -> Self {
        Self {
            tracker,
            _auth_params: auth_params,
            consensus,
            modules,
        }
    }

    // TODO: this will be needed when we restrict Resources
    // fn check_access_rules(&self, function: FunctionIdent, access_rules: &AccessRules) -> Result<(), RuntimeError> {
    //     // TODO: In this very basic auth system, you can only call on owned objects (because initial_ownership_proofs
    // is     //       usually set to include the owner token).
    //     let auth_zone = AuthorizationScope::new(&self.auth_params.initial_ownership_proofs);
    //     auth_zone.check_access_rules(&function, access_rules)
    // }

    fn invoke_on_runtime_call_modules(&self, function: &'static str) -> Result<(), RuntimeError> {
        for module in &self.modules {
            module.on_runtime_call(&self.tracker, function)?;
        }
        Ok(())
    }
}

impl RuntimeInterface for RuntimeInterfaceImpl {
    fn set_current_runtime_state(&self, state: RuntimeState) -> Result<(), RuntimeError> {
        self.invoke_on_runtime_call_modules("set_current_runtime_state")?;
        self.tracker.set_current_runtime_state(state);
        Ok(())
    }

    fn emit_log(&self, level: LogLevel, message: String) -> Result<(), RuntimeError> {
        self.invoke_on_runtime_call_modules("emit_log")?;

        let log_level = match level {
            LogLevel::Error => log::Level::Error,
            LogLevel::Warn => log::Level::Warn,
            LogLevel::Info => log::Level::Info,
            LogLevel::Debug => log::Level::Debug,
        };

        eprintln!("{}: {}", log_level, message);
        log::log!(target: "tari::dan::engine::runtime", log_level, "{}", message);
        self.tracker.add_log(LogEntry::new(level, message));
        Ok(())
    }

    fn get_component(&self, address: &ComponentAddress) -> Result<ComponentHeader, RuntimeError> {
        self.invoke_on_runtime_call_modules("get_component")?;
        self.tracker.get_component(address)
    }

    fn component_invoke(
        &self,
        component_ref: ComponentRef,
        action: ComponentAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError> {
        self.invoke_on_runtime_call_modules("component_invoke")?;

        match action {
            ComponentAction::Create => {
                let arg: CreateComponentArg = args.get(0)?;
                let template_def = self.tracker.get_template_def()?;
                validate_access_rules(&arg.access_rules, template_def)?;
                let component_address =
                    self.tracker
                        .new_component(arg.module_name, arg.encoded_state, arg.access_rules)?;
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
                Ok(InvokeResult::encode(&component)?)
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
        self.invoke_on_runtime_call_modules("resource_invoke")?;

        match action {
            ResourceAction::Create => {
                let arg: CreateResourceArg = args.get(0)?;

                let resource_address = self.tracker.new_resource(arg.resource_type, arg.metadata)?;

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

    fn vault_invoke(
        &self,
        vault_ref: VaultRef,
        action: VaultAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError> {
        self.invoke_on_runtime_call_modules("vault_invoke")?;

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
                    .borrow_vault_mut(&vault_id, |vault| vault.deposit(bucket))??;
                Ok(InvokeResult::unit())
            },
            VaultAction::Withdraw => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "WithdrawFungible vault action requires a vault id".to_string(),
                })?;
                let arg: VaultWithdrawArg = args.get(0)?;

                // TODO: access check
                let resource = self.tracker.borrow_vault_mut(&vault_id, |vault| match arg {
                    VaultWithdrawArg::Fungible { amount } => vault.withdraw(amount),
                    VaultWithdrawArg::NonFungible { ids } => vault.withdraw_non_fungibles(&ids),
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
                    .borrow_vault_mut(&vault_id, |vault| vault.withdraw_all())??;
                let bucket = self.tracker.new_bucket(resource)?;
                Ok(InvokeResult::encode(&bucket)?)
            },
            VaultAction::GetBalance => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "GetBalance vault action requires a vault id".to_string(),
                })?;
                // TODO: access check

                let balance = self.tracker.borrow_vault(&vault_id, |v| v.balance())?;
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
                    .borrow_vault_mut(&vault_id, |vault| *vault.resource_address())?;
                Ok(InvokeResult::encode(&address)?)
            },
            VaultAction::GetNonFungibleIds => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "vault action requires a vault id".to_string(),
                })?;

                // TODO: access check
                let resp = self.tracker.borrow_vault(&vault_id, |vault| {
                    let empty = BTreeSet::new();
                    let ids = vault.get_non_fungible_ids().unwrap_or(&empty);
                    // NOTE: A BTreeSet does not decode when received in the WASM
                    InvokeResult::encode(&ids.iter().collect::<Vec<_>>())
                })??;

                Ok(resp)
            },
        }
    }

    fn bucket_invoke(
        &self,
        bucket_ref: BucketRef,
        action: BucketAction,
        args: EngineArgs,
    ) -> Result<InvokeResult, RuntimeError> {
        self.invoke_on_runtime_call_modules("bucket_invoke")?;

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
        self.invoke_on_runtime_call_modules("workspace_invoke")?;
        match action {
            WorkspaceAction::ListBuckets => {
                let bucket_ids = self.tracker.list_buckets();
                Ok(InvokeResult::encode(&bucket_ids)?)
            },
            WorkspaceAction::Put => todo!(),
            WorkspaceAction::PutLastInstructionOutput => {
                let key = args.get(0)?;
                let last_output = self
                    .tracker
                    .take_last_instruction_output()
                    .ok_or(RuntimeError::NoLastInstructionOutput)?;
                self.tracker.put_in_workspace(key, last_output)?;
                Ok(InvokeResult::unit())
            },
            WorkspaceAction::Take => {
                let key: Vec<u8> = args.get(0)?;
                let value = self.tracker.take_from_workspace(&key)?;
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
        self.invoke_on_runtime_call_modules("non_fungible_invoke")?;
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
        self.invoke_on_runtime_call_modules("consensus_invoke")?;
        match action {
            ConsensusAction::GetCurrentEpoch => Ok(InvokeResult::encode(&self.consensus.current_epoch)?),
        }
    }

    fn generate_uuid(&self) -> Result<[u8; 32], RuntimeError> {
        self.invoke_on_runtime_call_modules("generate_uuid")?;
        let uuid = self.tracker.id_provider().new_uuid()?;
        Ok(uuid)
    }

    fn set_last_instruction_output(&self, value: Option<Vec<u8>>) -> Result<(), RuntimeError> {
        self.invoke_on_runtime_call_modules("set_last_instruction_output")?;
        self.tracker.set_last_instruction_output(value);
        Ok(())
    }

    fn finalize(&self) -> Result<FinalizeResult, RuntimeError> {
        self.invoke_on_runtime_call_modules("finalize")?;
        let result = match self.tracker.finalize() {
            Ok(substate_diff) => TransactionResult::Accept(substate_diff),
            Err(err) => TransactionResult::Reject(RejectReason::ExecutionFailure(err.to_string())),
        };
        let logs = self.tracker.take_logs();
        let commit = FinalizeResult::new(self.tracker.transaction_hash(), logs, result);

        Ok(commit)
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
