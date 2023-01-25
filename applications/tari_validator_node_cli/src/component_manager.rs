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

use std::{fs, io, path::Path};

use anyhow::anyhow;
use jfs::Config;
use serde::{Deserialize, Serialize};
use tari_dan_common_types::serde_with;
use tari_engine_types::substate::{SubstateAddress, SubstateDiff};

use crate::versioned_substate_address::VersionedSubstateAddress;

pub struct ComponentManager {
    store: jfs::Store,
}

impl ComponentManager {
    pub fn init<P: AsRef<Path>>(base_path: P) -> anyhow::Result<Self> {
        let path = base_path.as_ref().join("components");
        fs::create_dir_all(&path).map_err(|e| anyhow!("Failed to create component store dir: {}", e))?;
        let store = jfs::Store::new_with_cfg(path, Config {
            pretty: true,
            indent: 2,
            single: false,
        })
        .map_err(|e| anyhow!("Failed to create component store: {}", e))?;
        Ok(Self { store })
    }

    pub fn add_root_substate(
        &self,
        substate_addr: &SubstateAddress,
        version: u32,
        children: Vec<VersionedSubstateAddress>,
    ) -> anyhow::Result<()> {
        let substate = match self.get_root_substate(substate_addr)? {
            Some(mut substate) => {
                println!("Updating existing root substate: {} v{}", substate_addr, version);
                substate.versions.push((version, children));
                substate
            },
            None => SubstateMetadata {
                address: *substate_addr,
                versions: vec![(version, children)],
            },
        };

        self.store.save_with_id(&substate, &substate_addr.to_address_string())?;
        Ok(())
    }

    pub fn commit_diff(&self, diff: &SubstateDiff) -> anyhow::Result<()> {
        let mut component = None;
        let mut children = vec![];
        // for (addr, version) in diff.down_iter() {
        // self.remove_substate_version(addr, *version)?;
        // }

        for (addr, substate) in diff.up_iter() {
            match addr {
                addr @ SubstateAddress::Component(_) => {
                    component = Some((addr, substate.version()));
                },
                addr @ SubstateAddress::Resource(_) |
                addr @ SubstateAddress::Vault(_) |
                addr @ SubstateAddress::NonFungible(_, _) => {
                    children.push(VersionedSubstateAddress {
                        address: *addr,
                        version: substate.version(),
                    });
                },
            }
        }

        if let Some((addr, version)) = component {
            self.add_root_substate(addr, version, children)?;
        }
        Ok(())
    }

    // pub fn remove_substate_version(&self, address: &SubstateAddress, version: u32) -> anyhow::Result<()> {
    //     let mut substate = self
    //         .get_root_substate(address)?
    //         .ok_or_else(|| anyhow!("No substate found for address {}", address))?;
    //     let pos = substate
    //         .versions
    //         .iter()
    //         .position(|(v, _)| *v == version)
    //         .ok_or_else(|| anyhow!("No version {} found for substate {}", version, address))?;
    //     substate.versions.remove(pos);
    //     if substate.versions.is_empty() {
    //         self.store.delete(&address.to_address_string())?;
    //     } else {
    //         self.store.save_with_id(&substate, &address.to_address_string())?;
    //     }
    //     Ok(())
    // }

    pub fn get_root_substate(&self, substate_addr: &SubstateAddress) -> anyhow::Result<Option<SubstateMetadata>> {
        let meta = self.store.get(&substate_addr.to_address_string()).or_else(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                Ok(None)
            } else {
                Err(e)
            }
        })?;
        Ok(meta)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstateMetadata {
    #[serde(with = "serde_with::string")]
    pub address: SubstateAddress,
    pub versions: Vec<(u32, Vec<VersionedSubstateAddress>)>,
}

impl SubstateMetadata {
    pub fn latest_version(&self) -> u32 {
        self.versions.last().map(|(v, _)| *v).expect("versions is empty")
    }

    pub fn get_children(&self) -> Vec<VersionedSubstateAddress> {
        self.versions.last().map(|(_, c)| c.clone()).expect("versions is empty")
    }
}
