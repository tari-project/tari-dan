//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause
//

use std::{collections::HashMap, path::Path};

use borsh::BorshDeserialize;
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_dan_engine::{
    crypto::create_key_pair,
    packager::{LoadedTemplate, Package, TemplateModuleLoader},
    runtime::RuntimeInterface,
    state_store::{memory::MemoryStateStore, AtomicDb, StateReader, StateStoreError, StateWriter},
    transaction::{Transaction, TransactionError, TransactionProcessor},
    wasm::{compile::compile_template, LoadedWasmTemplate},
};
use tari_engine_types::{
    commit_result::FinalizeResult,
    hashing::hasher,
    instruction::Instruction,
    substate::{Substate, SubstateAddress, SubstateDiff},
};
use tari_template_lib::{
    args::Arg,
    models::{ComponentAddress, ComponentHeader, TemplateAddress},
};
use tari_transaction_manifest::{parse_manifest, ManifestValue};

use super::MockRuntimeInterface;

pub struct TemplateTest<R> {
    package: Package,
    processor: TransactionProcessor<R>,
    secret_key: RistrettoSecretKey,
    name_to_template: HashMap<String, TemplateAddress>,
    runtime_interface: R,
}

impl<R: RuntimeInterface + Clone + 'static> TemplateTest<R> {
    pub fn with_runtime_interface<P: AsRef<Path>>(templates: Vec<P>, runtime_interface: R) -> Self {
        let (secret_key, _pk) = create_key_pair();

        let wasms = templates.into_iter().map(|path| compile_template(path, &[]).unwrap());
        let mut builder = Package::builder();
        let mut name_to_template = HashMap::new();
        for wasm in wasms {
            let template_addr = hasher("test_template").chain(wasm.code()).result();
            let wasm = wasm.load_template().unwrap();
            let name = wasm.template_name().to_string();
            name_to_template.insert(name, template_addr);
            builder.add_template(template_addr, wasm);
        }
        let package = builder.build();
        let processor = TransactionProcessor::new(runtime_interface.clone(), package.clone());

        Self {
            package,
            processor,
            secret_key,
            name_to_template,
            runtime_interface,
        }
    }
}

impl TemplateTest<MockRuntimeInterface> {
    pub fn new<P: AsRef<Path>>(template_paths: Vec<P>) -> Self {
        let runtime_interface = MockRuntimeInterface::default();
        Self::with_runtime_interface(template_paths, runtime_interface)
    }

    pub fn read_only_state_store(&self) -> ReadOnlyStateStore {
        ReadOnlyStateStore::new(self.runtime_interface.state_store())
    }

    pub fn assert_calls(&self, expected: &[&'static str]) {
        let calls = self.runtime_interface.get_calls();
        assert_eq!(calls, expected);
    }

    pub fn clear_calls(&self) {
        self.runtime_interface.clear_calls();
    }

    fn commit_diff(&self, diff: &SubstateDiff) {
        let store = self.runtime_interface.state_store();
        let mut tx = store.write_access().unwrap();

        for (address, _) in diff.down_iter() {
            eprintln!("DOWN substate: {}", address);
            tx.delete_state(address).unwrap();
        }

        for (address, substate) in diff.up_iter() {
            eprintln!("UP substate: {}", address);
            tx.set_state(address, substate).unwrap();
        }

        tx.commit().unwrap();
    }

    pub fn get_module(&self, module_name: &str) -> &LoadedWasmTemplate {
        let addr = self.name_to_template.get(module_name).unwrap();
        match self.package.get_template_by_address(addr).unwrap() {
            LoadedTemplate::Wasm(wasm) => wasm,
        }
    }

    pub fn get_template_address(&self, name: &str) -> TemplateAddress {
        *self.name_to_template.get(name).unwrap()
    }

    pub fn call_function<T>(&self, template_name: &str, func_name: &str, args: Vec<Arg>) -> T
    where T: BorshDeserialize {
        let result = self.execute_and_commit(vec![Instruction::CallFunction {
            template_address: self.get_template_address(template_name),
            function: func_name.to_owned(),
            args,
        }]);
        result.execution_results[0].decode().unwrap()
    }

    pub fn call_method<T>(&self, component_address: ComponentAddress, method_name: &str, args: Vec<Arg>) -> T
    where T: BorshDeserialize {
        let result = self.execute_and_commit(vec![Instruction::CallMethod {
            component_address,
            method: method_name.to_owned(),
            args,
        }]);

        result.execution_results[0].decode().unwrap()
    }

    pub fn try_execute(&self, instructions: Vec<Instruction>) -> Result<FinalizeResult, TransactionError> {
        let mut builder = Transaction::builder();
        for instruction in instructions {
            builder.add_instruction(instruction);
        }
        builder.sign(&self.secret_key);
        let transaction = builder.build();

        self.processor.execute(transaction)
    }

    pub fn execute_and_commit(&self, instructions: Vec<Instruction>) -> FinalizeResult {
        let result = self.try_execute(instructions).unwrap();
        let diff = result
            .result
            .accept()
            .unwrap_or_else(|| panic!("Transaction was rejected: {}", result.result.reject().unwrap()));

        // It is convenient to commit the state back to the staged state store in tests.
        self.commit_diff(diff);

        result
    }

    pub fn execute_and_commit_manifest<'a, I: IntoIterator<Item = (&'a str, ManifestValue)>>(
        &self,
        manifest: &str,
        variables: I,
    ) -> FinalizeResult {
        let template_imports = self
            .name_to_template
            .iter()
            .map(|(name, addr)| format!("use template_{} as {};", addr, name))
            .collect::<Vec<_>>()
            .join("\n");
        let manifest = format!("{} fn main() {{ {} }}", template_imports, manifest);
        let instructions = parse_manifest(
            &manifest,
            variables.into_iter().map(|(a, b)| (a.to_string(), b)).collect(),
        )
        .unwrap();
        self.execute_and_commit(instructions)
    }
}

pub struct ReadOnlyStateStore {
    store: MemoryStateStore,
}
impl ReadOnlyStateStore {
    pub fn new(store: MemoryStateStore) -> Self {
        Self { store }
    }

    pub fn get_component(&self, component_address: ComponentAddress) -> Result<ComponentHeader, StateStoreError> {
        let tx = self.store.read_access()?;
        let substate = tx.get_state::<_, Substate>(&SubstateAddress::Component(component_address))?;
        Ok(substate.into_substate().into_component().unwrap())
    }
}
