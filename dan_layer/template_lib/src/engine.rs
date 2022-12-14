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

use tari_bor::{decode, encode, Decode, Encode};
use tari_template_abi::{call_engine, EngineOp};

use crate::{
    args::{ComponentAction, ComponentInvokeArg, ComponentRef, EmitLogArg, InvokeResult, LogLevel},
    context::Context,
    get_context,
    models::{ComponentAddress, ComponentHeader},
};

pub fn engine() -> TariEngine {
    // TODO: I expect some thread local state to be included here
    TariEngine::new(get_context())
}

#[derive(Debug, Default)]
pub struct TariEngine {
    _context: Context,
}

impl TariEngine {
    fn new(context: Context) -> Self {
        Self { _context: context }
    }

    pub fn instantiate<T: Encode>(&self, template_name: String, initial_state: T) -> ComponentAddress {
        let encoded_state = encode(&initial_state).unwrap();

        let result = call_engine::<_, InvokeResult>(EngineOp::ComponentInvoke, &ComponentInvokeArg {
            component_ref: ComponentRef::Component,
            action: ComponentAction::Create,
            args: invoke_args![template_name, encoded_state],
        });

        result.decode().expect("failed to decode component address")
    }

    pub fn emit_log<T: Into<String>>(&self, level: LogLevel, msg: T) {
        call_engine::<_, ()>(EngineOp::EmitLog, &EmitLogArg {
            level,
            message: msg.into(),
        });
    }

    /// Get the component state
    pub fn get_component_state<T: Decode>(&self, component_address: ComponentAddress) -> T {
        let result = call_engine::<_, InvokeResult>(EngineOp::ComponentInvoke, &ComponentInvokeArg {
            component_ref: ComponentRef::Ref(component_address),
            action: ComponentAction::Get,
            args: invoke_args![],
        });

        let component: ComponentHeader = result.decode().expect("failed to decode component header from engine");
        decode(component.state()).expect("Failed to decode component state")
    }

    pub fn set_component_state<T: Encode>(&self, component_address: ComponentAddress, state: T) {
        let state = encode(&state).expect("Failed to encode component state");
        let _result = call_engine::<_, InvokeResult>(EngineOp::ComponentInvoke, &ComponentInvokeArg {
            component_ref: ComponentRef::Ref(component_address),
            action: ComponentAction::SetState,
            args: invoke_args![state],
        });
    }
}
