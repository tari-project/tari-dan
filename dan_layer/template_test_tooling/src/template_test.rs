//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause
//

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use anyhow::anyhow;
use borsh::BorshDeserialize;
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_dan_engine::{
    crypto::create_key_pair,
    packager::{LoadedTemplate, Package, TemplateModuleLoader},
    state_store::{memory::MemoryStateStore, AtomicDb, StateReader, StateStoreError, StateWriter},
    transaction::{Transaction, TransactionError, TransactionProcessor},
    wasm::{compile::compile_template, LoadedWasmTemplate, WasmModule},
};
use tari_engine_types::{
    commit_result::FinalizeResult,
    hashing::hasher,
    instruction::Instruction,
    substate::{Substate, SubstateAddress, SubstateDiff},
};
use tari_template_builtin::{get_template_builtin, ACCOUNT_TEMPLATE_ADDRESS};
use tari_template_lib::{
    args::Arg,
    models::{ComponentAddress, ComponentHeader, TemplateAddress},
};
use tari_transaction_manifest::{parse_manifest, ManifestValue};

use super::MockRuntimeInterface;

pub struct TemplateTest<R> {
    package: Package,
    processor: TransactionProcessor<MockRuntimeInterface>,
    secret_key: RistrettoSecretKey,
    last_outputs: HashSet<SubstateAddress>,
    name_to_template: HashMap<String, TemplateAddress>,
    runtime_interface: R,
}

impl TemplateTest<MockRuntimeInterface> {
    pub fn new<P: AsRef<Path>>(template_paths: Vec<P>) -> Self {
        let runtime_interface = MockRuntimeInterface::default();
        let (secret_key, _pk) = create_key_pair();

        let wasms = template_paths
            .into_iter()
            .map(|path| compile_template(path, &[]).unwrap());
        let mut builder = Package::builder();
        let mut name_to_template = HashMap::new();

        // Add Account template builtin
        let wasm = get_template_builtin(&ACCOUNT_TEMPLATE_ADDRESS);
        let template = WasmModule::from_code(wasm.to_vec()).load_template().unwrap();
        builder.add_template(ACCOUNT_TEMPLATE_ADDRESS, template);
        name_to_template.insert("Account".to_string(), ACCOUNT_TEMPLATE_ADDRESS);

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
            last_outputs: HashSet::new(),
        }
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

    pub fn get_previous_output_address(&self, ty: SubstateType) -> SubstateAddress {
        self.last_outputs
            .iter()
            .find(|addr| ty.matches(addr))
            .cloned()
            .unwrap_or_else(|| panic!("No output of type {:?}", ty))
    }

    fn commit_diff(&mut self, diff: &SubstateDiff) {
        let store = self.runtime_interface.state_store();
        let mut tx = store.write_access().unwrap();
        self.last_outputs.clear();

        for (address, _) in diff.down_iter() {
            eprintln!("DOWN substate: {}", address);
            tx.delete_state(address).unwrap();
        }

        for (address, substate) in diff.up_iter() {
            eprintln!("UP substate: {}", address);
            self.last_outputs.insert(address.clone());
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
        *self
            .name_to_template
            .get(name)
            .unwrap_or_else(|| panic!("No template with name {}", name))
    }

    pub fn call_function<T>(&mut self, template_name: &str, func_name: &str, args: Vec<Arg>) -> T
    where T: BorshDeserialize {
        let result = self
            .execute_and_commit(vec![Instruction::CallFunction {
                template_address: self.get_template_address(template_name),
                function: func_name.to_owned(),
                args,
            }])
            .unwrap();
        result.execution_results[0].decode().unwrap()
    }

    pub fn call_method<T>(&mut self, component_address: ComponentAddress, method_name: &str, args: Vec<Arg>) -> T
    where T: BorshDeserialize {
        let result = self
            .execute_and_commit(vec![Instruction::CallMethod {
                component_address,
                method: method_name.to_owned(),
                args,
            }])
            .unwrap();

        result.execution_results[0].decode().unwrap()
    }

    pub fn try_execute(&mut self, instructions: Vec<Instruction>) -> Result<FinalizeResult, TransactionError> {
        let mut builder = Transaction::builder();
        for instruction in instructions {
            builder.add_instruction(instruction);
        }
        builder.sign(&self.secret_key);
        let transaction = builder.build();

        match self.processor.execute(transaction) {
            Ok(result) => Ok(result),
            Err(err) => {
                // If there's an error we want to clear state from the failed execution. This is equivalent to the
                // payload executor behaviour which sets up new state for each transaction.
                self.reset_runtime_state();
                Err(err)
            },
        }
    }

    fn reset_runtime_state(&mut self) {
        self.runtime_interface.reset_runtime();
        self.processor = TransactionProcessor::new(self.runtime_interface.clone(), self.package.clone());
    }

    pub fn execute_and_commit(&mut self, instructions: Vec<Instruction>) -> anyhow::Result<FinalizeResult> {
        let result = self.try_execute(instructions)?;
        let diff = result
            .result
            .accept()
            .ok_or_else(|| anyhow!("Transaction was rejected: {}", result.result.reject().unwrap()))?;

        // It is convenient to commit the state back to the staged state store in tests.
        self.commit_diff(diff);

        Ok(result)
    }

    pub fn execute_and_commit_manifest<'a, I: IntoIterator<Item = (&'a str, ManifestValue)>>(
        &mut self,
        manifest: &str,
        variables: I,
    ) -> anyhow::Result<FinalizeResult> {
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
        let substate = self.get_substate(&SubstateAddress::Component(component_address))?;
        Ok(substate.into_substate_value().into_component().unwrap())
    }

    pub fn get_substate(&self, address: &SubstateAddress) -> Result<Substate, StateStoreError> {
        let tx = self.store.read_access()?;
        let substate = tx.get_state::<_, Substate>(address)?;
        Ok(substate)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubstateType {
    Component,
    Resource,
    Vault,
    NonFungible,
}

impl SubstateType {
    pub fn matches(&self, addr: &SubstateAddress) -> bool {
        #[allow(clippy::match_like_matches_macro)]
        match (self, addr) {
            (SubstateType::Component, SubstateAddress::Component(_)) => true,
            (SubstateType::Resource, SubstateAddress::Resource(_)) => true,
            (SubstateType::Vault, SubstateAddress::Vault(_)) => true,
            (SubstateType::NonFungible, SubstateAddress::NonFungible(_, _)) => true,
            _ => false,
        }
    }
}
