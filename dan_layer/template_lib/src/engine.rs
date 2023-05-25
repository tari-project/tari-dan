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

use std::collections::HashMap;

use serde::Serialize;
use tari_bor::encode;
use tari_template_abi::{call_engine, EngineOp};

use crate::{
    args::{ComponentAction, ComponentInvokeArg, ComponentRef, CreateComponentArg, EmitLogArg, InvokeResult, LogLevel},
    component::ComponentManager,
    context::Context,
    events::emit_event,
    get_context,
    models::ComponentAddress,
    prelude::AccessRules,
    Hash,
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

    pub fn create_component<T: Serialize>(
        &self,
        module_name: String,
        initial_state: T,
        access_rules: AccessRules,
        component_id: Option<Hash>,
    ) -> ComponentAddress {
        let encoded_state = encode(&initial_state).unwrap();

        let result = call_engine::<_, InvokeResult>(
            EngineOp::ComponentInvoke,
            &ComponentInvokeArg {
                component_ref: ComponentRef::Component,
                action: ComponentAction::Create,
                args: invoke_args![CreateComponentArg {
                    module_name: module_name.clone(),
                    encoded_state,
                    access_rules: access_rules.clone(),
                    component_id,
                }],
            },
        );

        let component_address: ComponentAddress = result.decode().expect("failed to decode component address");
        let topic = "component-created".to_string();
        let payload = HashMap::from([
            ("component_address".to_string(), component_address.to_string()),
            ("module_name".to_string(), module_name),
            ("access_rules".to_string(), format!("{:?}", access_rules)),
        ]);
        emit_event(topic, payload);

        component_address
    }

    pub fn emit_log<T: Into<String>>(&self, level: LogLevel, msg: T) {
        call_engine::<_, ()>(
            EngineOp::EmitLog,
            &EmitLogArg {
                level,
                message: msg.into(),
            },
        );
    }

    pub fn component_manager(&self, component_address: ComponentAddress) -> ComponentManager {
        ComponentManager::new(component_address)
    }
}
