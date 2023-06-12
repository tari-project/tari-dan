//  Copyright 2023. The Tari Project
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

use tari_template_lib::prelude::*;

#[template]
mod composability {
    use super::*;

    pub struct Composability {
        // we assume the inner component is a "State" template component
        state_component_address: ComponentAddress
    }

    impl Composability {
        // function-to-function call
        // both "composability" and "state" components are created
        pub fn new(state_template_address: TemplateAddress) -> Self {
            let state_component_address = TemplateManager::get(state_template_address)
                .call("new".to_string(), vec![]);
            Self { state_component_address }
        }

        // function-to-component call
        // the argument is a "Composability" component, we get the "State" component address from it
        pub fn new_from_component(other_composability_component_address: ComponentAddress) -> Self {
            let state_component_address = ComponentManager::get(other_composability_component_address)
                .call("get_state_component_address".to_string(), vec![]);
            Self { state_component_address }
        }

        // component-to-component call
        pub fn increase_inner_state_component(&self) {
            let component = ComponentManager::get(self.state_component_address);

            // read operation, to get the current value of the inner "State" component
            let value: u32 = component
                .call("get".to_string(), vec![]);

            // write operation, to update the value of the inner "State" component
            component.call("set".to_string(), vec![value + 1]);
        }

        // function-to-component call
        pub fn replace_state_component(&mut self, state_template_address: TemplateAddress) {
            self.state_component_address = TemplateManager::get(state_template_address)
                .call("new".to_string(), vec![]);
        }

        pub fn get_state_component_address(&self) -> ComponentAddress {
            self.state_component_address
        }
    }
}
