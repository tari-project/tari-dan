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

use serde::{de::DeserializeOwned, Serialize};
use tari_bor::{from_value, to_value};
use tari_template_abi::{call_engine, EngineOp};

use crate::{
    args::{
        Arg,
        CallAction,
        CallInvokeArg,
        CallMethodArg,
        ComponentAction,
        ComponentInvokeArg,
        ComponentRef,
        InvokeResult,
    },
    auth::ComponentAccessRules,
    caller_context::CallerContext,
    models::{ComponentAddress, TemplateAddress},
};

pub struct ComponentManager {
    address: ComponentAddress,
}

impl ComponentManager {
    pub(crate) fn new(address: ComponentAddress) -> Self {
        Self { address }
    }

    pub fn get(address: ComponentAddress) -> Self {
        Self { address }
    }

    pub fn current() -> Self {
        Self::new(CallerContext::current_component_address())
    }

    pub fn call<T: Into<String>, R: DeserializeOwned>(&self, method: T, args: Vec<Arg>) -> R {
        self.call_internal(CallMethodArg {
            component_address: self.address,
            method: method.into(),
            args,
        })
    }

    fn call_internal<T: DeserializeOwned>(&self, arg: CallMethodArg) -> T {
        let result = call_engine::<_, InvokeResult>(EngineOp::CallInvoke, &CallInvokeArg {
            action: CallAction::CallMethod,
            args: invoke_args![arg],
        });

        result
            .decode()
            .expect("failed to decode component call result from engine")
    }

    /// Get the component state
    pub fn get_state<T: DeserializeOwned>(&self) -> T {
        let result = call_engine::<_, InvokeResult>(EngineOp::ComponentInvoke, &ComponentInvokeArg {
            component_ref: ComponentRef::Ref(self.address),
            action: ComponentAction::GetState,
            args: invoke_args![],
        });

        let component: tari_bor::Value = result.decode().expect("failed to decode component state from engine");
        from_value(&component).expect("Failed to decode component state")
    }

    pub fn set_state<T: Serialize>(&self, state: T) {
        let state = to_value(&state).expect("Failed to encode component state");
        let _result = call_engine::<_, InvokeResult>(EngineOp::ComponentInvoke, &ComponentInvokeArg {
            component_ref: ComponentRef::Ref(self.address),
            action: ComponentAction::SetState,
            args: invoke_args![state],
        });
    }

    pub fn set_access_rules(&self, access_rules: ComponentAccessRules) {
        call_engine::<_, InvokeResult>(EngineOp::ComponentInvoke, &ComponentInvokeArg {
            component_ref: ComponentRef::Ref(self.address),
            action: ComponentAction::SetAccessRules,
            args: invoke_args![access_rules],
        });
    }

    pub fn get_template_address(&self) -> TemplateAddress {
        let result = call_engine::<_, InvokeResult>(EngineOp::ComponentInvoke, &ComponentInvokeArg {
            component_ref: ComponentRef::Ref(self.address),
            action: ComponentAction::GetTemplateAddress,
            args: invoke_args![],
        });

        result
            .decode()
            .expect("failed to decode component template address from engine")
    }
}
