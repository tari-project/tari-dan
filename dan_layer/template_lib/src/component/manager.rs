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

use tari_bor::{decode_exact, Decode, Encode};
use tari_template_abi::{call_engine, EngineOp};

use crate::{
    args::{ComponentAction, ComponentInvokeArg, ComponentRef, InvokeResult, SetStateComponentArg},
    auth::AccessRules,
    models::{ComponentAddress, ComponentHeader},
    prelude::ComponentInterface,
};

pub struct ComponentManager {
    address: ComponentAddress,
}

impl ComponentManager {
    pub(crate) fn new(address: ComponentAddress) -> Self {
        Self { address }
    }

    /// Get the component state
    pub fn get_state<T: Decode>(&self) -> T {
        let result = call_engine::<_, InvokeResult>(EngineOp::ComponentInvoke, &ComponentInvokeArg {
            component_ref: ComponentRef::Ref(self.address),
            action: ComponentAction::Get,
            args: invoke_args![],
        });

        let component: ComponentHeader = result.decode().expect("failed to decode component header from engine");
        decode_exact(component.state()).expect("Failed to decode component state")
    }

    pub fn set_state<T: Encode + ComponentInterface>(&self, state: T) {
        let owned_values = state.get_owned_values();
        let _result = call_engine::<_, InvokeResult>(EngineOp::ComponentInvoke, &ComponentInvokeArg {
            component_ref: ComponentRef::Ref(self.address),
            action: ComponentAction::SetState,
            args: invoke_args![SetStateComponentArg { state, owned_values }],
        });
    }

    pub fn set_access_rules(&self, access_rules: AccessRules) {
        call_engine::<_, InvokeResult>(EngineOp::ComponentInvoke, &ComponentInvokeArg {
            component_ref: ComponentRef::Ref(self.address),
            action: ComponentAction::SetAccessRules,
            args: invoke_args![access_rules],
        });
    }
}
