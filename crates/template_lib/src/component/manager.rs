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
use minicbor::{CborLen, Decode, Encode};
use tari_template_abi::{EngineOp, call_engine, rust::prelude::*};
use tari_template_lib_types::{ComponentAddress, TemplateAddress, access_rules::ComponentAccessRules, bytes::Bytes};

use crate::{
    args::{CallAction, CallInvokeArg, CallMethodArg, ComponentAction, ComponentInvokeArg, ComponentRef, InvokeResult},
    caller_context::CallerContext,
    error_variants::ERR_ENGINE_DECODE_FAIL,
    models::Proof,
};

/// Utility for managing components inside templates
#[derive(Clone, Encode, Decode, CborLen)]
#[cbor(transparent)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(transparent))]
pub struct ComponentManager(#[n(0)] ComponentAddress);

impl ComponentManager {
    /// Returns a new `ComponentManager` for the component specified by `address`
    pub(crate) fn new(address: ComponentAddress) -> Self {
        Self(address)
    }

    /// Returns the address of the component that is being managed
    pub fn get(address: ComponentAddress) -> Self {
        Self(address)
    }

    /// Returns the address of the component that is being called in the current instruction.
    /// Assumes that the instruction is a call method; otherwise, it will panic
    pub fn current() -> Self {
        Self::new(CallerContext::current_component_address())
    }

    /// Calls a method of another component and returns the result.
    /// This is used to call external component methods and can be used in a component method or template function
    /// context.
    pub fn call<T: Into<String>, R: for<'b> Decode<'b, ()>, B: Into<Bytes>>(&self, method: T, args: Vec<B>) -> R {
        self.call_internal(CallMethodArg {
            component_address: self.0,
            method: method.into(),
            args: args.into_iter().map(Into::into).collect(),
        })
    }

    fn call_internal<T: for<'b> Decode<'b, ()>>(&self, arg: CallMethodArg) -> T {
        let result = call_engine::<_, InvokeResult>(EngineOp::CallInvoke, &CallInvokeArg {
            action: CallAction::CallMethod,
            args: invoke_args![arg],
        });

        result.decode().expect(ERR_ENGINE_DECODE_FAIL)
    }

    /// Calls a method of another component. The called method must return a unit type.
    /// Equivalent to [`call::<_, ()>(method, args)`](ComponentManager::call).
    pub fn invoke<T: Into<String>, B: Into<Bytes>>(&self, method: T, args: Vec<B>) {
        self.call(method, args)
    }

    /// Get the component state
    pub fn get_state<T: for<'b> Decode<'b, ()>>(&self) -> T {
        let result = call_engine::<_, InvokeResult>(EngineOp::ComponentInvoke, &ComponentInvokeArg {
            component_ref: ComponentRef::Ref(self.0),
            action: ComponentAction::GetState,
            args: invoke_args![],
        });

        result.decode().expect(ERR_ENGINE_DECODE_FAIL)
    }

    /// Gets the component state as a CBOR value enum
    pub fn get_state_value(&self) -> tari_bor::Value {
        self.get_state()
    }

    /// Update the component state
    pub fn set_state<T: Encode<()>>(&self, state: T) {
        let _result = call_engine::<_, InvokeResult>(EngineOp::ComponentInvoke, &ComponentInvokeArg {
            component_ref: ComponentRef::Ref(self.0),
            action: ComponentAction::SetState,
            args: invoke_args![state],
        });
    }

    /// Updates access rules that determine who can invoke methods in the component
    /// It will panic if the caller doesn't have permissions for updating access rules
    pub fn set_access_rules(&self, access_rules: ComponentAccessRules) {
        call_engine::<_, InvokeResult>(EngineOp::ComponentInvoke, &ComponentInvokeArg {
            component_ref: ComponentRef::Ref(self.0),
            action: ComponentAction::SetAccessRules,
            args: invoke_args![access_rules],
        });
    }

    /// Returns the template address of the component that is being managed
    pub fn get_template_address(&self) -> TemplateAddress {
        let result = call_engine::<_, InvokeResult>(EngineOp::ComponentInvoke, &ComponentInvokeArg {
            component_ref: ComponentRef::Ref(self.0),
            action: ComponentAction::GetTemplateAddress,
            args: invoke_args![],
        });

        result.decode().expect(ERR_ENGINE_DECODE_FAIL)
    }

    pub fn get_owner_proof(&self) -> Proof {
        let result = call_engine::<_, InvokeResult>(EngineOp::ComponentInvoke, &ComponentInvokeArg {
            component_ref: ComponentRef::Ref(self.0),
            action: ComponentAction::GetOwnerProof,
            args: invoke_args![],
        });

        result.decode().expect(ERR_ENGINE_DECODE_FAIL)
    }

    pub fn component_address(&self) -> ComponentAddress {
        self.0
    }
}
