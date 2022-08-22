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

use tari_template_abi::decode;
use tari_template_lib::{
    args::{
        BucketAction,
        BucketRef,
        CreateComponentArg,
        InvokeResult,
        LogLevel,
        MintResourceArg,
        ResourceAction,
        ResourceRef,
        VaultAction,
        WorkspaceAction,
    },
    models::{Amount, BucketId, ComponentAddress, ComponentInstance, VaultRef},
};

use crate::runtime::{
    logs::LogEntry,
    tracker::{RuntimeState, StateTracker},
    RuntimeError,
    RuntimeInterface,
};

#[derive(Debug, Clone)]
pub struct RuntimeInterfaceImpl {
    tracker: StateTracker,
}

impl RuntimeInterfaceImpl {
    pub fn new(tracker: StateTracker) -> Self {
        RuntimeInterfaceImpl { tracker }
    }
}

impl RuntimeInterface for RuntimeInterfaceImpl {
    fn set_current_runtime_state(&self, state: RuntimeState) {
        self.tracker.set_current_runtime_state(state);
    }

    fn emit_log(&self, level: LogLevel, message: String) {
        let log_level = match level {
            LogLevel::Error => log::Level::Error,
            LogLevel::Warn => log::Level::Warn,
            LogLevel::Info => log::Level::Info,
            LogLevel::Debug => log::Level::Debug,
        };

        log::log!(target: "tari::dan::engine::runtime", log_level, "{}", message);
        self.tracker.add_log(LogEntry::new(level, message));
    }

    fn create_component(&self, arg: CreateComponentArg) -> Result<ComponentAddress, RuntimeError> {
        let component_address = self.tracker.new_component(arg)?;
        Ok(component_address)
    }

    fn get_component(&self, component_address: &ComponentAddress) -> Result<ComponentInstance, RuntimeError> {
        let component = self.tracker.get_component(component_address)?;
        Ok(component)
    }

    fn set_component_state(&self, component_address: &ComponentAddress, state: Vec<u8>) -> Result<(), RuntimeError> {
        let mut component = self.tracker.get_component(component_address)?;
        // TODO: Need to validate this state somehow - it could contain arbitrary data incl. vaults that are not owned
        //       by this component
        component.state = state;
        self.tracker.set_component(component)?;
        Ok(())
    }

    fn resource_invoke(
        &self,
        _resource_ref: ResourceRef,
        action: ResourceAction,
        args: Vec<Vec<u8>>,
    ) -> Result<InvokeResult, RuntimeError> {
        match action {
            ResourceAction::Mint => {
                let mint_resource: MintResourceArg =
                    args.get(0)
                        .and_then(|r| decode(r).ok())
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "MintResourceArg",
                            reason: "Argument not provided or failed to decode".to_string(),
                        })?;

                let resource_address = self.tracker.mint_resource(mint_resource)?;
                Ok(InvokeResult::encode(&resource_address)?)
            },
            ResourceAction::Burn => todo!(),
            ResourceAction::Deposit => todo!(),
            ResourceAction::Withdraw => todo!(),
            ResourceAction::Update => todo!(),
        }
    }

    fn vault_invoke(
        &self,
        vault_ref: VaultRef,
        action: VaultAction,
        args: Vec<Vec<u8>>,
    ) -> Result<InvokeResult, RuntimeError> {
        match action {
            VaultAction::Create => {
                let resource_address = vault_ref
                    .resource_address()
                    .ok_or_else(|| RuntimeError::InvalidArgument {
                        argument: "vault_ref",
                        reason: "Create vault action requires a resource address".to_string(),
                    })?;
                let resource_type = vault_ref.resource_type().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "Create vault action requires a resource type".to_string(),
                })?;

                let vault_id = self.tracker.new_vault(*resource_address, resource_type);
                Ok(InvokeResult::encode(&vault_id)?)
            },
            VaultAction::Deposit => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "Put vault action requires a vault id".to_string(),
                })?;
                let bucket_id: BucketId =
                    args.get(0)
                        .and_then(|r| decode(r).ok())
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "bucket_id",
                            reason: "Argument not provided or failed to decode".to_string(),
                        })?;

                let bucket = self.tracker.take_bucket(bucket_id)?;
                let mut vault = self.tracker.get_vault(&vault_id)?;
                vault.deposit(bucket)?;
                self.tracker.set_vault(&vault_id, vault)?;
                Ok(InvokeResult::empty())
            },
            VaultAction::WithdrawFungible => {
                let vault_id = vault_ref.vault_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "vault_ref",
                    reason: "WithdrawFungible vault action requires a vault id".to_string(),
                })?;
                let amount: Amount =
                    args.get(0)
                        .and_then(|r| decode(r).ok())
                        .ok_or_else(|| RuntimeError::InvalidArgument {
                            argument: "amount",
                            reason: "Argument not provided or failed to decode".to_string(),
                        })?;
                let mut vault = self.tracker.get_vault(&vault_id)?;
                let resource = vault.withdraw(amount)?;
                let bucket = self.tracker.new_bucket(resource);
                self.tracker.set_vault(&vault_id, vault)?;
                Ok(InvokeResult::encode(&bucket)?)
            },
        }
    }

    fn bucket_invoke(
        &self,
        bucket_ref: BucketRef,
        action: BucketAction,
        args: Vec<Vec<u8>>,
    ) -> Result<InvokeResult, RuntimeError> {
        match action {
            BucketAction::Create => {
                let resource_address = bucket_ref
                    .resource_address()
                    .ok_or_else(|| RuntimeError::InvalidArgument {
                        argument: "bucket_ref",
                        reason: "Create bucket action requires a resource address".to_string(),
                    })?;
                let resource = self
                    .tracker
                    .get_resource(&resource_address)
                    .ok_or(RuntimeError::ResourceNotFound { resource_address })?;
                let bucket_id = self.tracker.new_bucket(resource);
                Ok(InvokeResult::encode(&bucket_id)?)
            },
            BucketAction::GetResourceAddress => {
                let bucket_id = bucket_ref.bucket_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "bucket_ref",
                    reason: "Create bucket action requires a bucket id".to_string(),
                })?;
                let bucket = self.tracker.get_bucket(bucket_id)?;
                Ok(InvokeResult::encode(&bucket.resource_address())?)
            },
            BucketAction::Take => {
                let bucket_id = bucket_ref.bucket_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "bucket_ref",
                    reason: "Take bucket action requires a bucket id".to_string(),
                })?;
                let amount = args
                    .get(0)
                    .and_then(|r| decode(r).ok())
                    .ok_or_else(|| RuntimeError::InvalidArgument {
                        argument: "amount",
                        reason: "Argument not provided or failed to decode".to_string(),
                    })?;
                let resource = self
                    .tracker
                    .with_bucket_mut(bucket_id, |bucket| bucket.take(amount))??;
                let bucket_id = self.tracker.new_bucket(resource);
                Ok(InvokeResult::encode(&bucket_id)?)
            },
            BucketAction::Drop => {
                let bucket_id = bucket_ref.bucket_id().ok_or_else(|| RuntimeError::InvalidArgument {
                    argument: "bucket_ref",
                    reason: "Create bucket action requires a bucket id".to_string(),
                })?;
                let bucket = self.tracker.take_bucket(bucket_id)?;
                if !bucket.amount().is_zero() {
                    return Err(RuntimeError::BucketNotEmpty { bucket_id });
                }
                Ok(InvokeResult::encode(&bucket.resource_address())?)
            },
        }
    }

    fn workspace_invoke(&self, action: WorkspaceAction, args: Vec<Vec<u8>>) -> Result<InvokeResult, RuntimeError> {
        match action {
            WorkspaceAction::Put => todo!(),
            WorkspaceAction::PutLastInstructionOutput => {
                let key = args.get(0).and_then(|r| decode::<Vec<u8>>(r).ok()).ok_or_else(|| {
                    RuntimeError::InvalidArgument {
                        argument: "key",
                        reason: "Argument not provided or failed to decode".to_string(),
                    }
                })?;
                let last_output = self
                    .tracker
                    .take_last_instruction_output()
                    .ok_or(RuntimeError::NoLastInstructionOutput)?;
                self.tracker.put_in_workspace(key, last_output)?;
                Ok(InvokeResult::empty())
            },
            WorkspaceAction::Take => {
                let key = args.get(0).and_then(|r| decode::<Vec<u8>>(r).ok()).ok_or_else(|| {
                    RuntimeError::InvalidArgument {
                        argument: "key",
                        reason: "Argument not provided or failed to decode".to_string(),
                    }
                })?;
                let value = self.tracker.take_from_workspace(&key)?;

                Ok(InvokeResult::encode(&value)?)
            },
        }
    }

    fn set_last_instruction_output(&self, value: Option<Vec<u8>>) -> Result<(), RuntimeError> {
        self.tracker.set_last_instruction_output(value);
        Ok(())
    }
}
