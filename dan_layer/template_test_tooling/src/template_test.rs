//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause
//

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use anyhow::anyhow;
use borsh::BorshDeserialize;
use tari_crypto::{ristretto::RistrettoSecretKey, tari_utilities::ByteArray};
use tari_dan_common_types::crypto::create_key_pair;
use tari_dan_engine::{
    bootstrap_state,
    packager::{LoadedTemplate, Package, TemplateModuleLoader},
    runtime::{AuthParams, ConsensusContext, RuntimeModule},
    state_store::{memory::MemoryStateStore, AtomicDb, StateReader, StateStoreError, StateWriter},
    transaction::{TransactionError, TransactionProcessor},
    wasm::{compile::compile_template, LoadedWasmTemplate, WasmModule},
};
use tari_engine_types::{
    commit_result::FinalizeResult,
    hashing::{hasher, EngineHashDomainLabel},
    instruction::Instruction,
    substate::{Substate, SubstateAddress, SubstateDiff},
};
use tari_template_builtin::{get_template_builtin, ACCOUNT_TEMPLATE_ADDRESS};
use tari_template_lib::{
    args,
    args::Arg,
    crypto::RistrettoPublicKeyBytes,
    models::{ComponentAddress, ComponentHeader, NonFungibleAddress, TemplateAddress},
};
use tari_transaction::Transaction;
use tari_transaction_manifest::{parse_manifest, ManifestValue};

use crate::track_calls::TrackCallsModule;

pub struct TemplateTest {
    package: Package,
    track_calls: TrackCallsModule,
    secret_key: RistrettoSecretKey,
    last_outputs: HashSet<SubstateAddress>,
    name_to_template: HashMap<String, TemplateAddress>,
    state_store: MemoryStateStore,
    // TODO: cleanup
    consensus_context: ConsensusContext,
}

impl TemplateTest {
    pub fn new<P: AsRef<Path>>(template_paths: Vec<P>) -> Self {
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
            let template_addr = hasher(EngineHashDomainLabel::Template).chain(wasm.code()).result();
            let wasm = wasm.load_template().unwrap();
            let name = wasm.template_name().to_string();
            name_to_template.insert(name, template_addr);
            builder.add_template(template_addr, wasm);
        }
        let package = builder.build();
        let state_store = MemoryStateStore::default();
        bootstrap_state(&mut state_store.write_access().unwrap()).unwrap();

        Self {
            package,
            track_calls: TrackCallsModule::new(),
            secret_key,
            name_to_template,
            last_outputs: HashSet::new(),
            state_store,
            // TODO: cleanup
            consensus_context: ConsensusContext { current_epoch: 0 },
        }
    }

    pub fn set_consensus_context(&mut self, consensus: ConsensusContext) {
        self.consensus_context = consensus;
    }

    pub fn read_only_state_store(&self) -> ReadOnlyStateStore {
        ReadOnlyStateStore::new(self.state_store.clone())
    }

    pub fn assert_calls(&self, expected: &[&'static str]) {
        let calls = self.track_calls.get();
        assert_eq!(calls, expected);
    }

    pub fn clear_calls(&self) {
        self.track_calls.clear();
    }

    pub fn get_previous_output_address(&self, ty: SubstateType) -> SubstateAddress {
        self.last_outputs
            .iter()
            .find(|addr| ty.matches(addr))
            .cloned()
            .unwrap_or_else(|| panic!("No output of type {:?}", ty))
    }

    fn commit_diff(&mut self, diff: &SubstateDiff) {
        self.last_outputs.clear();
        let mut tx = self.state_store.write_access().unwrap();

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
            LoadedTemplate::Flow(_) => {
                panic!("Not supported")
            },
        }
    }

    pub fn get_template_address(&self, name: &str) -> TemplateAddress {
        *self
            .name_to_template
            .get(name)
            .unwrap_or_else(|| panic!("No template with name {}", name))
    }

    pub fn call_function<T>(
        &mut self,
        template_name: &str,
        func_name: &str,
        args: Vec<Arg>,
        proofs: Vec<NonFungibleAddress>,
    ) -> T
    where
        T: BorshDeserialize,
    {
        let result = self
            .execute_and_commit(
                vec![Instruction::CallFunction {
                    template_address: self.get_template_address(template_name),
                    function: func_name.to_owned(),
                    args,
                }],
                proofs,
            )
            .unwrap();
        result.execution_results[0].decode().unwrap()
    }

    pub fn call_method<T>(
        &mut self,
        component_address: ComponentAddress,
        method_name: &str,
        args: Vec<Arg>,
        proofs: Vec<NonFungibleAddress>,
    ) -> T
    where
        T: BorshDeserialize,
    {
        let result = self
            .execute_and_commit(
                vec![Instruction::CallMethod {
                    component_address,
                    method: method_name.to_owned(),
                    args,
                }],
                proofs,
            )
            .unwrap();

        result.execution_results[0].decode().unwrap()
    }

    pub fn create_owned_account(&mut self) -> (ComponentAddress, NonFungibleAddress, RistrettoSecretKey) {
        let (owner_proof, secret_key) = self.create_owner_proof();
        let component = self.call_function("Account", "create", args![owner_proof], vec![owner_proof.clone()]);
        (component, owner_proof, secret_key)
    }

    pub fn create_owner_proof(&self) -> (NonFungibleAddress, RistrettoSecretKey) {
        let (secret_key, public_key) = create_key_pair();
        let public_key = RistrettoPublicKeyBytes::from_bytes(public_key.as_bytes()).unwrap();
        let owner_token = NonFungibleAddress::from_public_key(public_key);
        (owner_token, secret_key)
    }

    pub fn try_execute(
        &mut self,
        instructions: Vec<Instruction>,
        proofs: Vec<NonFungibleAddress>,
    ) -> Result<FinalizeResult, TransactionError> {
        todo!()
        // let mut builder = Transaction::builder();
        // for instruction in instructions {
        //     builder.add_instruction(instruction);
        // }
        // builder.sign(&self.secret_key);
        // let transaction = builder.build();
        //
        // let modules: Vec<Box<dyn RuntimeModule>> = vec![Box::new(self.track_calls.clone())];
        // let auth_params = AuthParams {
        //     initial_ownership_proofs: proofs,
        // };
        // let processor = TransactionProcessor::new(
        //     self.package.clone(),
        //     self.state_store.clone(),
        //     auth_params,
        //     self.consensus_context.clone(),
        //     modules,
        // );
        //
        // match processor.execute(transaction) {
        //     Ok(result) => Ok(result),
        //     Err(err) => Err(err),
        // }
    }

    pub fn execute_and_commit(
        &mut self,
        instructions: Vec<Instruction>,
        proofs: Vec<NonFungibleAddress>,
    ) -> anyhow::Result<FinalizeResult> {
        let result = self.try_execute(instructions, proofs)?;
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
        proofs: Vec<NonFungibleAddress>,
    ) -> anyhow::Result<FinalizeResult> {
        let template_imports = self
            .name_to_template
            .iter()
            // Account is implicitly imported.
            .filter(|(name, _)|* name != "Account")
            .map(|(name, addr)| format!("use template_{} as {};", addr, name))
            .collect::<Vec<_>>()
            .join("\n");
        let manifest = format!("{} fn main() {{ {} }}", template_imports, manifest);
        let instructions = parse_manifest(
            &manifest,
            variables.into_iter().map(|(a, b)| (a.to_string(), b)).collect(),
        )
        .unwrap();
        self.execute_and_commit(instructions, proofs)
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
    NonFungibleIndex,
}

impl SubstateType {
    pub fn matches(&self, addr: &SubstateAddress) -> bool {
        #[allow(clippy::match_like_matches_macro)]
        match (self, addr) {
            (SubstateType::Component, SubstateAddress::Component(_)) => true,
            (SubstateType::Resource, SubstateAddress::Resource(_)) => true,
            (SubstateType::Vault, SubstateAddress::Vault(_)) => true,
            (SubstateType::NonFungible, SubstateAddress::NonFungible(_)) => true,
            (SubstateType::NonFungibleIndex, SubstateAddress::NonFungibleIndex(_)) => true,
            _ => false,
        }
    }
}
