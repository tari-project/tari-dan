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

use std::{fs, path::Path};

use tari_engine_types::substate::{SubstateAddress, SubstateDiff};
use tari_template_lib::models::ComponentAddress;
use tari_utilities::hex::to_hex;

pub struct ComponentManager {
    store: jfs::Store,
}

impl ComponentManager {
    pub fn init<P: AsRef<Path>>(base_path: P) -> anyhow::Result<Self> {
        let path = base_path.as_ref().join("components");
        fs::create_dir_all(&path)?;
        let store = jfs::Store::new(path)?;
        Ok(Self { store })
    }

    pub fn add_component(
        &self,
        component_address: &ComponentAddress,
        children: Vec<SubstateAddress>,
    ) -> anyhow::Result<()> {
        self.store.save_with_id(&children, &to_hex(component_address))?;
        Ok(())
    }

    pub fn commit_diff(&self, diff: &SubstateDiff) -> anyhow::Result<()> {
        let mut component_addr = None;
        let mut children = vec![];
        for (addr, _) in diff.up_iter() {
            match addr {
                SubstateAddress::Component(addr) => {
                    component_addr = Some(addr);
                },
                SubstateAddress::Resource(resx) => {
                    children.push(SubstateAddress::Resource(*resx));
                },
                SubstateAddress::Vault(vault_id) => {
                    children.push(SubstateAddress::Vault(*vault_id));
                },
            }
        }

        if let Some(addr) = component_addr {
            self.add_component(addr, children)?;
        }
        Ok(())
    }

    // pub fn remove_component(&self, address: ComponentAddress) -> anyhow::Result<()> {
    //     self.store.delete(&to_hex(&address))?;
    //     Ok(())
    // }

    pub fn get_component_childen(&self, address: &ComponentAddress) -> anyhow::Result<Vec<SubstateAddress>> {
        let children = self.store.get::<Vec<SubstateAddress>>(&to_hex(address))?;
        Ok(children)
    }
}
