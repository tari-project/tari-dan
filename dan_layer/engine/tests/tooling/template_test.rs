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

use std::path::Path;

use borsh::BorshDeserialize;
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_dan_engine::{
    crypto::create_key_pair,
    instruction::{Instruction, InstructionProcessor, TransactionBuilder},
    packager::Package,
    runtime::{CommitResult, RuntimeInterface},
    state_store::memory::MemoryStateStore,
    wasm::{compile::compile_template, LoadedWasmModule},
};
use tari_template_lib::{
    args::Arg,
    models::{ComponentAddress, PackageAddress},
};

use super::MockRuntimeInterface;

pub struct TemplateTest<R> {
    package: Package,
    processor: InstructionProcessor<R>,
    secret_key: RistrettoSecretKey,
    runtime_interface: R,
}

impl TemplateTest<MockRuntimeInterface> {
    pub fn new<P: AsRef<Path>>(template_paths: Vec<P>) -> Self {
        let runtime_interface = MockRuntimeInterface::new();
        Self::with_runtime_interface(template_paths, runtime_interface)
    }

    pub fn state_store(&self) -> MemoryStateStore {
        self.runtime_interface.state_store()
    }

    pub fn assert_calls(&self, expected: &[&'static str]) {
        let calls = self.runtime_interface.get_calls();
        assert_eq!(calls, expected);
    }

    pub fn clear_calls(&self) {
        self.runtime_interface.clear_calls();
    }
}

impl<R: RuntimeInterface + Clone + 'static> TemplateTest<R> {
    pub fn with_runtime_interface<P: AsRef<Path>>(template_paths: Vec<P>, runtime_interface: R) -> Self {
        let (secret_key, _pk) = create_key_pair();

        let wasms = template_paths
            .into_iter()
            .map(|path| compile_template(path, &[]).unwrap());
        let mut builder = Package::builder();
        for wasm in wasms {
            builder.add_wasm_module(wasm);
        }
        let package = builder.build().unwrap();
        let processor = InstructionProcessor::new(runtime_interface.clone(), package.clone());

        Self {
            package,
            processor,
            secret_key,
            runtime_interface,
        }
    }

    pub fn get_module(&self, module_name: &str) -> &LoadedWasmModule {
        self.package.get_module_by_name(module_name).unwrap()
    }

    pub fn package_address(&self) -> PackageAddress {
        self.package.address()
    }

    pub fn call_function<T>(&self, template_name: &str, func_name: &str, args: Vec<Arg>) -> T
    where T: BorshDeserialize {
        let result = self.execute(vec![Instruction::CallFunction {
            package_address: self.package.address(),
            template: template_name.to_owned(),
            function: func_name.to_owned(),
            args,
        }]);

        result.execution_results[0].decode::<T>().unwrap()
    }

    pub fn call_method<T>(&self, component_address: ComponentAddress, method_name: &str, args: Vec<Arg>) -> T
    where T: BorshDeserialize {
        let result = self.execute(vec![Instruction::CallMethod {
            package_address: self.package.address(),
            component_address,
            method: method_name.to_owned(),
            args,
        }]);

        result.execution_results[0].decode::<T>().unwrap()
    }

    pub fn execute(&self, instructions: Vec<Instruction>) -> CommitResult {
        let mut builder = TransactionBuilder::new();
        for instruction in instructions {
            builder.add_instruction(instruction);
        }
        let transaction = builder.sign(&self.secret_key).build();

        self.processor.execute(transaction).unwrap()
    }
}
