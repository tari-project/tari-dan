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

use std::{collections::HashMap, ops::DerefMut};

use rand::{rngs::OsRng, RngCore};
use tari_template_lib::models::PackageAddress;

use crate::{
    hashing::hasher,
    packager::{error::PackageError, PackageModuleLoader},
    wasm::{LoadedWasmModule, WasmModule},
};

#[derive(Debug, Clone)]
pub struct Package {
    id: PackageAddress,
    wasm_modules: HashMap<String, LoadedWasmModule>,
}

impl Package {
    pub fn builder() -> PackageBuilder {
        PackageBuilder::new()
    }

    pub fn get_module_by_name(&self, name: &str) -> Option<&LoadedWasmModule> {
        self.wasm_modules.get(name)
    }

    pub fn id(&self) -> PackageAddress {
        self.id
    }
}

#[derive(Debug, Clone, Default)]
pub struct PackageBuilder {
    wasm_modules: Vec<WasmModule>,
}

impl PackageBuilder {
    pub fn new() -> Self {
        Self {
            wasm_modules: Vec::new(),
        }
    }

    pub fn add_wasm_module(&mut self, wasm_module: WasmModule) -> &mut Self {
        self.wasm_modules.push(wasm_module);
        self
    }

    pub fn build(&self) -> Result<Package, PackageError> {
        let mut wasm_modules = HashMap::with_capacity(self.wasm_modules.len());
        for wasm in &self.wasm_modules {
            let loaded = wasm.load_module()?;
            wasm_modules.insert(loaded.template_name().to_string(), loaded);
        }
        let id = new_package_address(wasm_modules.values());

        Ok(Package { id, wasm_modules })
    }
}

fn new_package_address<'a, I: IntoIterator<Item = &'a LoadedWasmModule>>(modules: I) -> PackageAddress {
    let nonce = OsRng.next_u32();
    let mut hasher = hasher("package").chain(&nonce);
    for module in modules {
        hasher.update(&module.template_def());
    }
    let mut hash = hasher.result();
    hash.deref_mut()[..4].copy_from_slice(&nonce.to_le_bytes());
    hash
}
