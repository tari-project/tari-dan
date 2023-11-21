//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
    time::Instant,
};

use anyhow::anyhow;
use serde::de::DeserializeOwned;
use tari_bor::{decode_exact, to_value};
use tari_crypto::{
    keys::PublicKey,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    tari_utilities::{hex::Hex, ByteArray},
};
use tari_dan_common_types::crypto::create_key_pair;
use tari_dan_engine::{
    bootstrap_state,
    fees::{FeeModule, FeeTable},
    runtime::{AuthParams, RuntimeModule, VirtualSubstates},
    state_store::{
        memory::{MemoryStateStore, MemoryWriteTransaction},
        AtomicDb,
        StateWriter,
    },
    template::{LoadedTemplate, TemplateModuleLoader},
    transaction::{TransactionError, TransactionProcessor},
    wasm::{LoadedWasmTemplate, WasmModule},
};
use tari_engine_types::{
    commit_result::{ExecuteResult, RejectReason},
    component::{ComponentBody, ComponentHeader},
    instruction::Instruction,
    resource_container::ResourceContainer,
    substate::{Substate, SubstateAddress, SubstateDiff},
    vault::Vault,
    virtual_substate::{VirtualSubstate, VirtualSubstateAddress},
};
use tari_template_builtin::{get_template_builtin, ACCOUNT_TEMPLATE_ADDRESS};
use tari_template_lib::{
    args,
    args::Arg,
    auth::OwnerRule,
    crypto::RistrettoPublicKeyBytes,
    models::{Amount, ComponentAddress, NonFungibleAddress, TemplateAddress},
    prelude::{ComponentAccessRules, CONFIDENTIAL_TARI_RESOURCE_ADDRESS},
    Hash,
};
use tari_transaction::{id_provider::IdProvider, Transaction};
use tari_transaction_manifest::{parse_manifest, ManifestValue};

use crate::{read_only_state_store::ReadOnlyStateStore, track_calls::TrackCallsModule, Package};

pub fn test_faucet_component() -> ComponentAddress {
    ComponentAddress::new(Hash::from_array([0xfau8; 32]))
}

pub struct TemplateTest {
    package: Arc<Package>,
    track_calls: TrackCallsModule,
    secret_key: RistrettoSecretKey,
    public_key: RistrettoPublicKey,
    last_outputs: HashSet<SubstateAddress>,
    name_to_template: HashMap<String, TemplateAddress>,
    state_store: MemoryStateStore,
    enable_fees: bool,
    fee_table: FeeTable,
    virtual_substates: VirtualSubstates,
}

impl TemplateTest {
    pub fn new<I: IntoIterator<Item = P>, P: AsRef<Path>>(template_paths: I) -> Self {
        let mut builder = Package::builder();
        // Add Account template builtin
        let wasm = get_template_builtin(&ACCOUNT_TEMPLATE_ADDRESS);
        let template = WasmModule::from_code(wasm.to_vec()).load_template().unwrap();
        builder.add_loaded_template(*ACCOUNT_TEMPLATE_ADDRESS, template);

        builder.add_template(concat!(env!("CARGO_MANIFEST_DIR"), "/templates/faucet"));
        for path in template_paths {
            builder.add_template(path);
        }

        let package = builder.build();

        let test = Self::from_package(package);
        test.bootstrap_faucet(100_000.into());
        test
    }

    pub fn from_package(package: Package) -> Self {
        let secret_key =
            RistrettoSecretKey::from_hex("8a39567509bf2f7074e5fd153337405292cdc9f574947313b62fbf8fb4cffc02").unwrap();

        let public_key = RistrettoPublicKey::from_secret_key(&secret_key);

        let mut name_to_template = HashMap::new();

        for (addr, template) in package.iter() {
            if name_to_template
                .insert(template.template_name().to_string(), *addr)
                .is_some()
            {
                panic!("Duplicate template name: {}", template.template_name());
            }
        }

        let state_store = MemoryStateStore::default();
        {
            let mut tx = state_store.write_access().unwrap();
            bootstrap_state(&mut tx).unwrap();
            tx.commit().unwrap();
        }

        let mut virtual_substates = VirtualSubstates::new();
        virtual_substates.insert(VirtualSubstateAddress::CurrentEpoch, VirtualSubstate::CurrentEpoch(0));

        Self {
            package: Arc::new(package),
            track_calls: TrackCallsModule::new(),
            public_key,
            secret_key,
            name_to_template,
            last_outputs: HashSet::new(),
            state_store,
            virtual_substates,
            enable_fees: false,
            fee_table: FeeTable {
                per_module_call_cost: 1,
                per_byte_storage_cost: 1,
                per_event_cost: 1,
                per_log_cost: 1,
            },
        }
    }

    pub fn bootstrap_faucet(&self, amount: Amount) {
        let mut tx = self.state_store.write_access().unwrap();
        Self::initial_tari_faucet_supply(
            &mut tx,
            &self.public_key,
            amount,
            self.get_template_address("TestFaucet"),
        );
        tx.commit().unwrap();
    }

    fn initial_tari_faucet_supply(
        tx: &mut MemoryWriteTransaction<'_>,
        signer_public_key: &RistrettoPublicKey,
        initial_supply: Amount,
        test_faucet_template_address: TemplateAddress,
    ) {
        let id_provider = IdProvider::new(Hash::default(), 10);
        let vault_id = id_provider.new_vault_id().unwrap();
        let vault = Vault::new(
            vault_id,
            ResourceContainer::confidential(CONFIDENTIAL_TARI_RESOURCE_ADDRESS, vec![], initial_supply),
        );
        tx.set_state(&SubstateAddress::Vault(vault_id), Substate::new(0, vault))
            .unwrap();

        // This must mirror the test faucet component
        #[derive(serde::Serialize)]
        struct Faucet {
            vault: tari_template_lib::models::Vault,
        }
        let state = to_value(&Faucet {
            vault: tari_template_lib::models::Vault::for_test(vault_id),
        })
        .unwrap();
        tx.set_state(
            &SubstateAddress::Component(test_faucet_component()),
            Substate::new(0, ComponentHeader {
                template_address: test_faucet_template_address,
                module_name: "TestFaucet".to_string(),
                owner_key: RistrettoPublicKeyBytes::from_bytes(signer_public_key.as_bytes()).unwrap(),
                owner_rule: OwnerRule::None,
                access_rules: ComponentAccessRules::allow_all(),
                body: ComponentBody { state },
            }),
        )
        .unwrap();
    }

    pub fn enable_fees(&mut self) -> &mut Self {
        self.enable_fees = true;
        self
    }

    pub fn disable_fees(&mut self) -> &mut Self {
        self.enable_fees = false;
        self
    }

    pub fn fee_table(&self) -> &FeeTable {
        &self.fee_table
    }

    pub fn set_fee_table(&mut self, fee_table: FeeTable) -> &mut Self {
        self.fee_table = fee_table;
        self
    }

    pub fn set_virtual_substate(&mut self, address: VirtualSubstateAddress, value: VirtualSubstate) -> &mut Self {
        self.virtual_substates.insert(address, value);
        self
    }

    pub fn read_only_state_store(&self) -> ReadOnlyStateStore {
        ReadOnlyStateStore::new(self.state_store.clone())
    }

    pub fn extract_component_value<T: DeserializeOwned>(&self, component_address: ComponentAddress, path: &str) -> T {
        self.read_only_state_store()
            .inspect_component(component_address)
            .unwrap()
            .get_value(path)
            .unwrap()
            .unwrap()
    }

    pub fn default_signing_key(&self) -> &RistrettoSecretKey {
        &self.secret_key
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
        T: DeserializeOwned,
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
        result.finalize.execution_results[0].decode().unwrap()
    }

    pub fn call_method<T>(
        &mut self,
        component_address: ComponentAddress,
        method_name: &str,
        args: Vec<Arg>,
        proofs: Vec<NonFungibleAddress>,
    ) -> T
    where
        T: DeserializeOwned,
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

        result.finalize.execution_results[0].decode().unwrap()
    }

    pub fn get_instructions_to_pay_fee_from_faucet(&self) -> Vec<Instruction> {
        vec![Instruction::CallFunction {
            template_address: self.get_template_address("Faucet2"),
            function: "pay_fee_confidential".to_string(),
            args: args![],
        }]
    }

    pub fn get_test_proof_and_secret_key(&self) -> (NonFungibleAddress, RistrettoSecretKey) {
        (self.get_test_proof(), self.secret_key.clone())
    }

    pub fn get_test_proof(&self) -> NonFungibleAddress {
        NonFungibleAddress::from_public_key(self.get_test_public_key_bytes())
    }

    pub fn get_test_secret_key(&self) -> &RistrettoSecretKey {
        &self.secret_key
    }

    pub fn get_test_public_key(&self) -> &RistrettoPublicKey {
        &self.public_key
    }

    pub fn get_test_public_key_bytes(&self) -> RistrettoPublicKeyBytes {
        RistrettoPublicKeyBytes::from_bytes(self.public_key.as_bytes()).unwrap()
    }

    pub fn create_empty_account(&mut self) -> (ComponentAddress, NonFungibleAddress, RistrettoSecretKey) {
        let (owner_proof, secret_key) = self.create_owner_proof();
        let old_fail_fees = self.enable_fees;
        self.enable_fees = false;
        let component = self.call_function("Account", "create", args![owner_proof], vec![owner_proof.clone()]);
        self.enable_fees = old_fail_fees;
        (component, owner_proof, secret_key)
    }

    pub fn create_owned_account(&mut self) -> (ComponentAddress, NonFungibleAddress, RistrettoSecretKey) {
        let (owner_proof, secret_key) = self.create_owner_proof();
        let old_fail_fees = self.enable_fees;
        self.enable_fees = false;
        let result = self.execute_expect_success(
            Transaction::builder()
                .call_method(test_faucet_component(), "take_free_coins", args![])
                .put_last_instruction_output_on_workspace("bucket")
                .call_function(self.get_template_address("Account"), "create_with_bucket", args![
                    &owner_proof,
                    Workspace("bucket")
                ])
                .sign(&secret_key)
                .build(),
            vec![owner_proof.clone()],
        );

        let component = result.finalize.execution_results[2]
            .decode::<ComponentAddress>()
            .unwrap();

        self.enable_fees = old_fail_fees;
        (component, owner_proof, secret_key)
    }

    pub fn create_owner_proof(&self) -> (NonFungibleAddress, RistrettoSecretKey) {
        let (secret_key, public_key) = create_key_pair();
        let public_key = RistrettoPublicKeyBytes::from_bytes(public_key.as_bytes()).unwrap();
        let owner_token = NonFungibleAddress::from_public_key(public_key);
        (owner_token, secret_key)
    }

    pub fn try_execute_instructions(
        &mut self,
        fee_instructions: Vec<Instruction>,
        instructions: Vec<Instruction>,
        proofs: Vec<NonFungibleAddress>,
    ) -> Result<ExecuteResult, TransactionError> {
        let transaction = Transaction::builder()
            .with_fee_instructions(fee_instructions)
            .with_instructions(instructions)
            .sign(&self.secret_key)
            .build();

        self.try_execute(transaction, proofs)
    }

    pub fn try_execute(
        &mut self,
        transaction: Transaction,
        proofs: Vec<NonFungibleAddress>,
    ) -> Result<ExecuteResult, TransactionError> {
        let mut modules: Vec<Arc<dyn RuntimeModule>> = vec![Arc::new(self.track_calls.clone())];

        if self.enable_fees {
            modules.push(Arc::new(FeeModule::new(0, self.fee_table.clone())));
        }

        let auth_params = AuthParams {
            initial_ownership_proofs: proofs,
        };
        let processor = TransactionProcessor::new(
            self.package.clone(),
            self.state_store.clone(),
            auth_params,
            self.virtual_substates.clone(),
            modules,
        );

        let tx_id = *transaction.id();
        eprintln!("START Transaction id = \"{}\"", tx_id);

        let result = processor.execute(transaction)?;

        if self.enable_fees {
            if let Some(ref fee) = result.fee_receipt {
                eprintln!("Initial payment: {}", fee.total_allocated_fee_payments());
                eprintln!("Fee: {}", fee.total_fees_charged());
                eprintln!("Paid: {}", fee.total_fees_paid());
                eprintln!("Refund: {}", fee.total_refunded());
                eprintln!("Unpaid: {}", fee.unpaid_debt());
                for (source, amt) in &fee.cost_breakdown {
                    eprintln!("- {:?} {}", source, amt);
                }
            }
        }

        let timer = Instant::now();
        eprintln!("Finished Transaction \"{}\" in {:.2?}", tx_id, timer.elapsed());
        eprintln!();

        Ok(result)
    }

    pub fn execute_and_commit_on_success(
        &mut self,
        transaction: Transaction,
        proofs: Vec<NonFungibleAddress>,
    ) -> ExecuteResult {
        let result = self.try_execute(transaction, proofs).unwrap();
        if let Some(diff) = result.finalize.result.accept() {
            self.commit_diff(diff);
        }

        result
    }

    /// Executes a transaction. Panics if the transaction is not finalized (fee transaction fails). Does not panic if
    /// the main instructions fails (use execute_expect_success for that).
    pub fn execute_expect_commit(
        &mut self,
        transaction: Transaction,
        proofs: Vec<NonFungibleAddress>,
    ) -> ExecuteResult {
        let result = self.try_execute(transaction, proofs).unwrap();
        let diff = result.expect_finalization_success();
        self.commit_diff(diff);

        result
    }

    /// Executes a transaction. Panics if the transaction fails.
    pub fn execute_expect_success(
        &mut self,
        transaction: Transaction,
        proofs: Vec<NonFungibleAddress>,
    ) -> ExecuteResult {
        let result = self.execute_expect_commit(transaction, proofs);
        result.expect_success();
        result
    }

    /// Executes a transaction. Panics if the transaction succeeds.
    pub fn execute_expect_failure(
        &mut self,
        transaction: Transaction,
        proofs: Vec<NonFungibleAddress>,
    ) -> RejectReason {
        let result = self.try_execute(transaction, proofs).unwrap();
        result.expect_failure().clone()
    }

    pub fn execute_and_commit(
        &mut self,
        instructions: Vec<Instruction>,
        proofs: Vec<NonFungibleAddress>,
    ) -> anyhow::Result<ExecuteResult> {
        self.execute_and_commit_with_fees(vec![], instructions, proofs)
    }

    pub fn execute_and_commit_with_fees(
        &mut self,
        fee_instructions: Vec<Instruction>,
        instructions: Vec<Instruction>,
        proofs: Vec<NonFungibleAddress>,
    ) -> anyhow::Result<ExecuteResult> {
        let result = self.try_execute_instructions(fee_instructions, instructions, proofs)?;
        let diff = result
            .finalize
            .result
            .accept()
            .ok_or_else(|| anyhow!("Transaction was rejected: {}", result.finalize.result.reject().unwrap()))?;

        // It is convenient to commit the state back to the staged state store in tests.
        self.commit_diff(diff);

        if let Some(reason) = result.finalize.full_reject() {
            return Err(anyhow!("Transaction failed: {}", reason));
        }

        Ok(result)
    }

    pub fn execute_and_commit_manifest<'a, I: IntoIterator<Item = (&'a str, ManifestValue)>>(
        &mut self,
        manifest: &str,
        variables: I,
        proofs: Vec<NonFungibleAddress>,
    ) -> anyhow::Result<ExecuteResult> {
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

    pub fn print_state(&self) {
        let tx = self.state_store.read_access().unwrap();
        for (k, v) in tx.iter_raw() {
            let k: SubstateAddress = decode_exact(k).unwrap();
            let v: Substate = decode_exact(v).unwrap();

            eprintln!("[{}]: {}", k, v.into_substate_value());
        }
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
