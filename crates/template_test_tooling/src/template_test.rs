//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    env,
    ffi::OsStr,
    iter,
    path::Path,
    sync::Arc,
    time::Instant,
};

use anyhow::anyhow;
use ootle_byte_type::ToByteType;
use serde::de::DeserializeOwned;
use tari_crypto::{
    keys::PublicKey as _,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    tari_utilities::hex::Hex,
};
use tari_engine::{
    executables::Executable,
    fees::{FeeModule, FeeTable, WasmMeteringRate},
    runtime::{AuthParams, RuntimeModule},
    state_store::{
        StateWriter,
        memory::{MemoryStateStore, ReadOnlyMemoryStateStore},
    },
    template::LoadedTemplate,
    transaction::{TransactionError, TransactionProcessor},
    wasm::LoadedWasmTemplate,
};
use tari_engine_types::{
    commit_result::{ExecuteResult, RejectReason},
    fees::ExhaustBurnRate,
    substate::{SubstateDiff, SubstateId},
    virtual_substate::{VirtualSubstate, VirtualSubstateId},
};
use tari_ootle_common_types::{
    Epoch,
    SubstateRequirement,
    crypto::create_key_pair_from_seed,
    substate_type::SubstateType,
};
use tari_ootle_transaction::{
    Instruction,
    Network,
    Transaction,
    TransactionBuilder,
    args,
    builder::{
        MainIntent,
        named_args::{BuilderWorkspaceKey, NamedArg},
    },
};
use tari_template_lib::types::{
    ComponentAddress,
    NonFungibleAddress,
    TemplateAddress,
    constants::{NFT_FAUCET_COMPONENT_ADDRESS, XTR_FAUCET_COMPONENT_ADDRESS},
    crypto::RistrettoPublicKeyBytes,
};
use tari_transaction_manifest::{ManifestValue, parse_manifest};

use crate::{
    Package,
    builtin_component_state::{
        add_tari_resources,
        initialize_builtin_faucet_state,
        initialize_builtin_nft_faucet_state,
    },
    helpers::derive_account_address_from_public_key,
    mocks::AlwaysPassesProofVerifier,
    read_only_state_store::ReadOnlyStateStore,
    template_spec::TemplateSpec,
    track_calls::TrackCallsModule,
    wrapped_transaction::WrappedTransaction,
};

/// Returns the component address of the built-in XTR (Tari) faucet used in tests.
pub const fn xtr_faucet_component() -> ComponentAddress {
    XTR_FAUCET_COMPONENT_ADDRESS
}

/// Returns the component address of the built-in NFT faucet used in tests.
pub fn test_nft_faucet_component() -> ComponentAddress {
    NFT_FAUCET_COMPONENT_ADDRESS
}

/// Test harness for Tari Ootle templates.
///
/// Compiles WASM templates, manages an in-memory state store, and provides convenience methods
/// for executing transactions, creating accounts, and inspecting results. Designed for use in
/// `#[test]` functions within template crates.
///
/// # Quick start
///
/// ```rust,no_run
/// use tari_template_test_tooling::TemplateTest;
///
/// let mut test = TemplateTest::my_crate();
/// let (account, proof, secret_key) = test.create_funded_account();
/// ```
pub struct TemplateTest {
    package: Arc<Package>,
    track_calls: TrackCallsModule,
    secret_key: RistrettoSecretKey,
    public_key: RistrettoPublicKey,
    last_outputs: HashSet<SubstateId>,
    last_execution_points: ExecutionPoints,
    name_to_template: HashMap<String, TemplateAddress>,
    state_store: MemoryStateStore,
    enable_fees: bool,
    dry_run: bool,
    fee_table: FeeTable,
    burn_rate: ExhaustBurnRate,
    virtual_substates: HashMap<VirtualSubstateId, VirtualSubstate>,
    key_seed: u8,
    auto_add_proofs_from_signers: bool,
    /// Uniquifies each transaction built via [`Self::transaction`]. The transaction id excludes
    /// the seal signature, so two transactions with identical bodies sealed by the same test key
    /// are the *same* transaction — and would collide on id-derived addresses. Feeding this to the
    /// nonce keeps their bodies distinct, and keeps them reproducible across runs the way a random
    /// nonce would not.
    transaction_seq: Cell<u64>,
}

/// The metering points a transaction consumed, split the way the engine charges them.
///
/// Both halves are real CPU on every validator and are priced identically, so a test reasoning
/// about what a transaction costs a validator wants [`Self::total`]; the split matters only when a
/// test is attributing cost to WASM execution or to native crypto specifically.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExecutionPoints {
    /// Wasmer metering points, bounded per transaction by `limits::MAX_WASM_POINTS_PER_TRANSACTION`.
    pub wasm: u64,
    /// Native verification points (stealth transfers, confidential withdraws, burn claims), priced
    /// by `limits::NativeExecutionPoints` and bounded by `limits::MAX_NATIVE_POINTS_PER_TRANSACTION`.
    pub native: u64,
}

impl ExecutionPoints {
    /// Every point the transaction cost a validator, WASM and native alike.
    pub fn total(&self) -> u64 {
        self.wasm.saturating_add(self.native)
    }
}

impl TemplateTest {
    /// The initial balance of a funded account created by `create_funded_account`.
    pub const FUNDED_ACCOUNT_INITIAL_BALANCE: u64 = 1_000_000_000;

    /// Creates a new `TemplateTest` with the template in the current crate. This is useful for tests within a template
    /// crate.
    pub fn my_crate() -> Self {
        Self::new(".", ["."])
    }

    /// Creates a new `TemplateTest` with templates relative to the given base path.
    /// All template_paths are resolved relative to the base path.
    pub fn new<P: AsRef<Path>, I: IntoIterator<Item = T>, T: Into<TemplateSpec>>(
        base_path: P,
        template_paths: I,
    ) -> Self {
        Self::new_internal(base_path, template_paths, iter::empty::<(String, String)>())
    }

    /// Creates a new `TemplateTest` using the current working directory as the base path.
    /// Template paths are resolved relative to the CWD.
    pub fn new_cwd<I: IntoIterator<Item = T>, T: Into<TemplateSpec>>(template_paths: I) -> Self {
        Self::new_internal(
            env::current_dir().expect("cannot get CWD"),
            template_paths,
            None::<(String, String)>,
        )
    }

    /// Creates a new `TemplateTest` with only the built-in templates (e.g. `Account`, faucets).
    /// No user templates are compiled or loaded.
    pub fn new_builtin_only() -> Self {
        Self::new(".", iter::empty::<TemplateSpec>())
    }

    /// Creates a new `TemplateTest` with additional environment variables set during WASM compilation.
    /// This is useful for templates that use `env!()` or conditional compilation based on environment variables.
    pub fn new_with_compile_envs<P, I, T, TEnvs, K, V>(base_path: P, template_paths: I, envs: TEnvs) -> Self
    where
        P: AsRef<Path>,
        I: IntoIterator<Item = T>,
        T: Into<TemplateSpec>,
        TEnvs: IntoIterator<Item = (K, V)>,
        TEnvs::IntoIter: Clone,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        Self::new_internal(base_path, template_paths, envs)
    }

    fn new_internal<P, I, T, TEnvs, K, V>(base_path: P, templates: I, envs: TEnvs) -> Self
    where
        P: AsRef<Path>,
        I: IntoIterator<Item = T>,
        T: Into<TemplateSpec>,
        TEnvs: IntoIterator<Item = (K, V)>,
        TEnvs::IntoIter: Clone,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        let mut builder = Package::builder();

        builder.add_all_builtin_templates();

        // Add all of the templates specified in the argument
        let envs_iter = envs.into_iter();
        let base_path = base_path.as_ref();
        for template in templates {
            let spec = template.into();
            builder.add_template_opts(spec.get_path(base_path), &spec.features, envs_iter.clone());
        }

        let package = builder.build();

        let mut test = Self::from_package(package);
        test.bootstrap_state();
        test
    }

    /// Builds a harness over a package assembled by the caller. The public constructors compile
    /// templates from source; this is the seam for a caller that already has loaded templates —
    /// notably one embedding pre-compiled WASM so it needs no toolchain at run time.
    pub fn from_package(package: Package) -> Self {
        let secret_key =
            RistrettoSecretKey::from_hex("8a39567509bf2f7074e5fd153337405292cdc9f574947313b62fbf8fb4cffc02").unwrap();

        let public_key = RistrettoPublicKey::from_secret_key(&secret_key);

        let mut name_to_template = HashMap::new();

        for (addr, template) in &package.templates() {
            if name_to_template
                .insert(template.template_name().to_string(), *addr)
                .is_some()
            {
                panic!("Duplicate template name: {}", template.template_name());
            }
        }

        let virtual_substates = HashMap::from_iter([
            (VirtualSubstateId::CurrentEpoch, VirtualSubstate::CurrentEpoch(0)),
            (
                VirtualSubstateId::CurrentEpochHash,
                VirtualSubstate::CurrentEpochHash(Default::default()),
            ),
        ]);

        Self {
            package: Arc::new(package),
            track_calls: TrackCallsModule::new(),
            public_key,
            secret_key,
            name_to_template,
            last_outputs: HashSet::new(),
            state_store: MemoryStateStore::new(),
            virtual_substates,
            last_execution_points: ExecutionPoints::default(),
            transaction_seq: Cell::new(0),
            enable_fees: false,
            dry_run: false,
            fee_table: FeeTable {
                per_transaction_weight_cost: 1,
                per_module_call_cost: 1,
                per_byte_storage_cost: 1,
                per_template_load_cost_unit: 1,
                per_substate_create_cost: 1,
                per_wasm_point_cost: 1,
                storage_cost_divisor: 1,
                template_load_bytes_cost_divisor: 3000,
                wasm_points_cost_divisor: 1000,
                log_bytes_cost_divisor: 64,
                template_size_premium_free_bytes: 96 * 1024,
                template_size_premium_unit_bytes: 1024,
                per_template_size_premium_unit_cost: 100,
                per_template_publish_cost: 250_000,
            },
            burn_rate: ExhaustBurnRate::new(0),
            key_seed: 1,
            auto_add_proofs_from_signers: true,
        }
    }

    /// Initializes the in-memory state store with built-in resources and faucet state.
    /// This is called automatically by the constructors and typically does not need to be called manually.
    pub fn bootstrap_state(&mut self) {
        add_tari_resources(&mut self.state_store).unwrap();
        initialize_builtin_faucet_state(&mut self.state_store);
        initialize_builtin_nft_faucet_state(&mut self.state_store)
    }

    /// Compiles and adds a new template to the test environment after initial construction.
    /// Returns the [`TemplateAddress`] assigned to the newly compiled template.
    /// The template is registered under the given `name` for later lookup via
    /// [`get_template_address`](Self::get_template_address).
    pub fn compile_new_template<T, P, TEnvs, K, V>(
        &mut self,
        name: T,
        path: P,
        features: &[&str],
        envs: TEnvs,
    ) -> TemplateAddress
    where
        T: Into<String>,
        P: AsRef<Path>,
        TEnvs: IntoIterator<Item = (K, V)>,
        TEnvs::IntoIter: Clone,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        let mut builder = Package::builder();
        for (addr, template) in self.package.templates() {
            builder.add_loaded_template(addr, template);
        }
        let template_addr = builder.add_template_opts(path, features, envs);
        self.package = Arc::new(builder.build());
        self.name_to_template.insert(name.into(), template_addr);

        template_addr
    }

    /// Executes subsequent transactions as a dry run: fees are metered but not settled, as the
    /// indexer's fee estimation does.
    pub fn set_dry_run(&mut self, dry_run: bool) -> &mut Self {
        self.dry_run = dry_run;
        self
    }

    /// Enables fee charging for subsequent transaction executions.
    /// By default, fees are disabled in tests.
    pub fn enable_fees(&mut self) -> &mut Self {
        self.enable_fees = true;
        self
    }

    /// Disables fee charging for subsequent transaction executions.
    pub fn disable_fees(&mut self) -> &mut Self {
        self.enable_fees = false;
        self
    }

    /// Enables automatic proof generation from transaction signers.
    /// When enabled (the default), if the `proofs` argument is empty, proofs are automatically
    /// derived from the transaction's signing keys.
    pub fn enable_auto_add_proofs_from_signers(&mut self) -> &mut Self {
        self.auto_add_proofs_from_signers = true;
        self
    }

    /// Disables automatic proof generation from transaction signers.
    /// When disabled, you must explicitly pass the required proofs to each execution call.
    pub fn disable_auto_add_proofs_from_signers(&mut self) -> &mut Self {
        self.auto_add_proofs_from_signers = false;
        self
    }

    /// Returns a reference to the current fee table used when fees are enabled.
    pub fn fee_table(&self) -> &FeeTable {
        &self.fee_table
    }

    /// Replaces the fee table with the given one. Only has effect when fees are enabled.
    pub fn set_fee_table(&mut self, fee_table: FeeTable) -> &mut Self {
        self.fee_table = fee_table;
        self
    }

    /// Sets the exhaust burn rate applied to the transaction's accrued fees. Defaults to zero, so
    /// tests see no burn unless they ask for one.
    pub fn set_burn_rate_bps(&mut self, rate_bps: u16) -> &mut Self {
        self.burn_rate = ExhaustBurnRate::new(rate_bps);
        self
    }

    /// Sets a virtual substate (e.g. `CurrentEpoch`) that is available to transactions during execution.
    pub fn set_virtual_substate(&mut self, address: VirtualSubstateId, value: VirtualSubstate) -> &mut Self {
        self.virtual_substates.insert(address, value);
        self
    }

    /// Removes a virtual substate so that it is not available to transactions during execution.
    ///
    /// This is useful for testing that templates handle missing virtual substates correctly (e.g.
    /// asserting that `Consensus::current_epoch_hash()` returns `VirtualSubstateNotFound` when the
    /// hash has not been injected).
    pub fn remove_virtual_substate(&mut self, address: VirtualSubstateId) -> &mut Self {
        self.virtual_substates.remove(&address);
        self
    }

    /// Returns a read-only view of the current state store, useful for inspecting component state
    /// between transactions.
    pub fn read_only_state_store(&self) -> ReadOnlyStateStore<'_> {
        ReadOnlyStateStore::new(&self.state_store)
    }

    pub fn get_state_store_mut(&mut self) -> &mut MemoryStateStore {
        &mut self.state_store
    }

    /// Extracts and deserializes a value from a component's state at the given JSON pointer `path`.
    ///
    /// # Panics
    ///
    /// Panics if the component does not exist, the path is invalid, or the value cannot be deserialized
    /// into `T`.
    pub fn extract_component_value<T>(&self, component_address: ComponentAddress, path: &str) -> T
    where T: DeserializeOwned + for<'b> tari_bor::Decode<'b, ()> {
        self.read_only_state_store()
            .inspect_component(component_address)
            .unwrap()
            .get_value(path)
            .unwrap()
            .unwrap_or_else(|| panic!("Expected component to have value at '{path}' but no value was found"))
    }

    /// Returns the default secret key used to sign transactions when no other key is specified.
    pub fn default_signing_key(&self) -> &RistrettoSecretKey {
        &self.secret_key
    }

    /// Asserts that the tracked cross-template calls match the given expected list exactly.
    ///
    /// # Panics
    ///
    /// Panics if the recorded calls do not match `expected`.
    #[track_caller]
    pub fn assert_calls(&self, expected: &[&'static str]) {
        let calls = self.track_calls.get();
        assert_eq!(calls, expected);
    }

    /// Clears the tracked cross-template call log.
    pub fn clear_calls(&self) {
        self.track_calls.clear();
    }

    /// Returns a [`SubstateId`] from the outputs of the most recently committed transaction
    /// that matches the given [`SubstateType`].
    ///
    /// # Panics
    ///
    /// Panics if no output of the given type was produced by the last transaction.
    pub fn get_previous_output_address(&self, ty: SubstateType) -> SubstateId {
        self.last_outputs
            .iter()
            .find(|addr| ty.matches(addr))
            .cloned()
            .unwrap_or_else(|| panic!("No output of type {:?}", ty))
    }

    /// The execution points the most recently executed transaction consumed, whether it was
    /// accepted or rejected.
    ///
    /// [`ExecuteResult`] already carries these, but the convenience callers — [`Self::call_function`],
    /// [`Self::call_method`] — return only the decoded value, so this is how a test reads the cost of
    /// a call it made through them. Zero if no transaction has executed yet, or if the last one
    /// failed before producing a result.
    pub fn last_execution_points(&self) -> ExecutionPoints {
        self.last_execution_points
    }

    fn commit_diff(&mut self, diff: &SubstateDiff) {
        self.last_outputs.clear();

        eprintln!("State changes:");
        for (address, _) in diff.down_iter() {
            eprintln!("DOWN substate: {}", address);
            self.state_store.delete_state(address);
        }

        for (address, substate) in diff.up_iter() {
            eprintln!("UP substate: {}", address);
            self.last_outputs.insert(address.clone());
            self.state_store.set_state(address.clone(), substate.clone()).unwrap();
        }
        eprintln!();
    }

    /// Returns the compiled WASM module for the template registered under the given name.
    ///
    /// # Panics
    ///
    /// Panics if no template with the given name exists.
    pub fn get_module(&self, module_name: &str) -> LoadedWasmTemplate {
        let addr = self.name_to_template.get(module_name).unwrap();
        match self.package.get_template_by_address(addr).unwrap() {
            LoadedTemplate::Wasm(wasm) => wasm,
        }
    }

    /// Returns the [`TemplateAddress`] for the template registered under the given name.
    ///
    /// # Panics
    ///
    /// Panics if no template with the given name exists.
    pub fn get_template_address(&self, name: &str) -> TemplateAddress {
        *self
            .name_to_template
            .get(name)
            .unwrap_or_else(|| panic!("No template with name {}", name))
    }

    /// Creates a new account component owned by the given public key.
    /// Returns the [`ComponentAddress`] of the newly created account.
    ///
    /// Optionally places the result on the workspace under `workspace_id`.
    /// Additional `proofs` are passed as initial ownership proofs for the transaction.
    #[track_caller]
    pub fn create_account(
        &mut self,
        owner_public_key: RistrettoPublicKeyBytes,
        workspace_id: Option<BuilderWorkspaceKey>,
        proofs: Vec<NonFungibleAddress>,
    ) -> ComponentAddress {
        let result = self
            .build_and_execute(
                self.transaction()
                    .create_account_custom(owner_public_key, None, None, workspace_id),
                proofs,
            )
            .unwrap_success();
        let diff = result.finalize.accept().expect("create account failed");
        diff.up_iter().find_map(|(id, _)| id.as_component_address()).unwrap()
    }

    /// Calls a template function by name and returns the deserialized result.
    ///
    /// This is a convenience method that builds a transaction with a single `CallFunction` instruction,
    /// executes it, and decodes the return value.
    ///
    /// # Panics
    ///
    /// Panics if the transaction fails or if the return value cannot be deserialized into `T`.
    #[track_caller]
    pub fn call_function<T>(
        &mut self,
        template_name: &str,
        func_name: &str,
        args: Vec<NamedArg>,
        proofs: Vec<NonFungibleAddress>,
    ) -> T
    where
        T: DeserializeOwned + for<'b> tari_bor::Decode<'b, ()>,
    {
        let address = self.get_template_address(template_name);
        let result = self.execute_expect_success(
            self.transaction()
                .call_function(address, func_name, args)
                .build_and_seal(&self.secret_key),
            proofs,
        );
        result
            .finalize
            .execution_results
            .first()
            .expect("single instruction without execution result")
            .decode()
            .unwrap()
    }

    /// Calls a method on an existing component and returns the deserialized result.
    ///
    /// This is a convenience method that builds a transaction with a single `CallMethod` instruction,
    /// executes it, and decodes the return value.
    ///
    /// # Panics
    ///
    /// Panics if the transaction fails or if the return value cannot be deserialized into `T`.
    #[track_caller]
    pub fn call_method<T>(
        &mut self,
        component_address: ComponentAddress,
        method_name: &str,
        args: Vec<NamedArg>,
        proofs: Vec<NonFungibleAddress>,
    ) -> T
    where
        T: DeserializeOwned + for<'b> tari_bor::Decode<'b, ()>,
    {
        let result = self.execute_expect_success(
            self.transaction()
                .call_method(component_address, method_name, args)
                .build_and_seal(&self.secret_key),
            proofs,
        );

        result
            .finalize
            .execution_results
            .first()
            .expect("single instruction without execution result")
            .decode()
            .unwrap()
    }

    /// Returns the default owner proof (non-fungible address) and secret key pair.
    /// Useful for setting up ownership proofs in tests.
    pub fn get_test_proof_and_secret_key(&self) -> (NonFungibleAddress, RistrettoSecretKey) {
        (self.owner_proof(), self.secret_key.clone())
    }

    /// Returns the default owner proof derived from the test's default public key.
    pub fn owner_proof(&self) -> NonFungibleAddress {
        NonFungibleAddress::from_public_key(self.public_key.to_byte_type())
    }

    /// Returns a reference to the default secret key.
    pub fn secret_key(&self) -> &RistrettoSecretKey {
        &self.secret_key
    }

    /// Returns a reference to the default public key.
    pub fn public_key(&self) -> &RistrettoPublicKey {
        &self.public_key
    }

    /// Generates a deterministic key pair from the given seed byte.
    /// Different seeds produce different key pairs, allowing tests to create multiple distinct identities.
    pub fn new_key_pair(&mut self, seed: u8) -> (RistrettoSecretKey, RistrettoPublicKey) {
        create_key_pair_from_seed(seed)
    }

    /// Returns the default public key as [`RistrettoPublicKeyBytes`], the byte representation
    /// commonly used in template function arguments.
    pub fn to_public_key_bytes(&self) -> RistrettoPublicKeyBytes {
        self.public_key.to_byte_type()
    }

    /// Creates a new account with zero balance and a fresh key pair.
    /// Returns `(account_address, owner_proof, secret_key)`.
    ///
    /// Fees are temporarily disabled for the account creation transaction.
    #[track_caller]
    pub fn create_empty_account(&mut self) -> (ComponentAddress, NonFungibleAddress, RistrettoSecretKey) {
        let (owner_proof, public_key, secret_key) = self.create_owner_proof();
        let old_fail_fees = self.enable_fees;
        self.enable_fees = false;
        let component = self.create_account(public_key.to_byte_type(), None, vec![owner_proof.clone()]);
        self.enable_fees = old_fail_fees;
        (component, owner_proof, secret_key)
    }

    /// Creates a new account funded with [`FUNDED_ACCOUNT_INITIAL_BALANCE`](Self::FUNDED_ACCOUNT_INITIAL_BALANCE)
    /// tokens from the XTR faucet, using a fresh key pair.
    /// Returns `(account_address, owner_proof, secret_key)`.
    ///
    /// Fees are temporarily disabled for the account creation transaction.
    #[track_caller]
    pub fn create_funded_account(&mut self) -> (ComponentAddress, NonFungibleAddress, RistrettoSecretKey) {
        let (owner_proof, public_key, secret_key) = self.create_owner_proof();
        let old_fail_fees = self.enable_fees;
        self.enable_fees = false;
        self.execute_expect_success(
            self.transaction()
                .create_account(public_key.to_byte_type())
                .put_last_instruction_output_on_workspace("account")
                .call_method(xtr_faucet_component(), "take", args![Workspace("account")])
                .build_and_seal(&secret_key),
            vec![owner_proof.clone()],
        );

        let account_address = derive_account_address_from_public_key(&public_key.to_byte_type());

        self.enable_fees = old_fail_fees;
        (account_address, owner_proof, secret_key)
    }

    /// Creates a new account funded from the XTR faucet using a fresh key pair.
    /// Returns `(account_address, owner_proof, secret_key, public_key)`.
    ///
    /// Unlike [`create_funded_account`], this also returns the public key.
    /// Fees are temporarily disabled for the account creation transaction.
    #[track_caller]
    pub fn create_funded_account_with_keypair(
        &mut self,
    ) -> (
        ComponentAddress,
        NonFungibleAddress,
        RistrettoSecretKey,
        RistrettoPublicKey,
    ) {
        let (owner_proof, public_key, secret_key) = self.create_owner_proof();
        let old_fail_fees = self.enable_fees;
        self.enable_fees = false;
        let public_key_bytes = public_key.to_byte_type();
        self.execute_expect_success(
            self.transaction()
                .create_account(public_key_bytes)
                .put_last_instruction_output_on_workspace("account")
                .call_method(xtr_faucet_component(), "take", args![Workspace("account")])
                .build_and_seal(&secret_key),
            vec![owner_proof.clone()],
        );

        let component = derive_account_address_from_public_key(&public_key_bytes);

        self.enable_fees = old_fail_fees;
        (component, owner_proof, secret_key, public_key)
    }

    fn next_key_seed(&mut self) -> u8 {
        let seed = self.key_seed;
        self.key_seed += 1;
        seed
    }

    /// Creates a fresh owner proof by generating a new key pair with an auto-incrementing seed.
    /// Returns `(owner_proof, public_key, secret_key)`.
    ///
    /// Each call produces a different key pair, making this suitable for creating multiple distinct owners.
    #[track_caller]
    pub fn create_owner_proof(&mut self) -> (NonFungibleAddress, RistrettoPublicKey, RistrettoSecretKey) {
        let (secret_key, public_key) = create_key_pair_from_seed(self.next_key_seed());
        let owner_token = NonFungibleAddress::from_public_key(public_key.to_byte_type());
        (owner_token, public_key, secret_key)
    }

    /// Builds and executes a transaction from raw fee and main instruction vectors.
    /// Returns `Ok(ExecuteResult)` on successful execution, or a [`TransactionError`] if the
    /// transaction processor encounters a fatal error.
    ///
    /// Unlike [`execute_expect_success`](Self::execute_expect_success), this does not panic on
    /// rejection and does not commit state changes.
    #[track_caller]
    pub fn try_execute_instructions(
        &mut self,
        fee_instructions: Vec<Instruction>,
        instructions: Vec<Instruction>,
        proofs: Vec<NonFungibleAddress>,
    ) -> Result<ExecuteResult, TransactionError> {
        let transaction = self
            .transaction()
            .with_fee_instructions(fee_instructions)
            .with_instructions(instructions)
            .build_and_seal(&self.secret_key);

        self.try_execute(transaction, proofs)
    }

    /// Executes a pre-built transaction without committing state changes.
    /// Returns `Ok(ExecuteResult)` on successful execution, or a [`TransactionError`] if the
    /// transaction processor encounters a fatal error.
    ///
    /// This is the lowest-level execution method. It does not panic on transaction rejection
    /// and does not commit the resulting state diff. Use this when you need full control over
    /// result handling.
    #[track_caller]
    pub fn try_execute(
        &mut self,
        transaction: Transaction,
        mut proofs: Vec<NonFungibleAddress>,
    ) -> Result<ExecuteResult, TransactionError> {
        let mut modules: Vec<Box<dyn RuntimeModule<ReadOnlyMemoryStateStore>>> = Vec::with_capacity(2);

        modules.push(Box::new(self.track_calls.clone()));

        // When fees are disabled there is no payment-funded compute bound (only the per-transaction
        // hard cap), matching the absence of a fee module.
        let wasm_metering_rate = if self.enable_fees {
            modules.push(Box::new(FeeModule::new(0, self.fee_table.clone())));
            WasmMeteringRate::from_fee_table(&self.fee_table)
        } else {
            WasmMeteringRate::unmetered()
        };

        if self.auto_add_proofs_from_signers && proofs.is_empty() {
            proofs.extend(
                transaction
                    .signers_iter()
                    .map(|pk| NonFungibleAddress::from_public_key(*pk)),
            );
        }

        let auth_params = AuthParams {
            initial_ownership_proofs: proofs.into_iter().collect(),
        };

        let processor = TransactionProcessor::new(
            self.package.clone(),
            self.state_store.clone().into_read_only(),
            auth_params,
            self.virtual_substates.clone().into(),
            Arc::from(modules.into_boxed_slice()),
            Arc::new(AlwaysPassesProofVerifier),
            wasm_metering_rate,
            self.burn_rate,
            Network::LocalNet,
            self.dry_run,
        );

        let mut wrapped_transaction = WrappedTransaction::new(transaction);
        // Add all the substates as inputs - this avoids the need for tests to explicitly include inputs
        wrapped_transaction.extend_inputs(
            self.state_store
                .iter()
                .map(|(id, s)| SubstateRequirement::versioned(id.clone(), s.version())),
        );

        let tx_id = wrapped_transaction.to_id();
        eprintln!("START Transaction id = \"{}\"", tx_id);

        let result = processor.execute(wrapped_transaction).inspect_err(|_| {
            self.last_execution_points = ExecutionPoints::default();
        })?;
        self.last_execution_points = ExecutionPoints {
            wasm: result.wasm_execution_points,
            native: result.native_execution_points,
        };

        if self.enable_fees {
            let fee = &result.finalize.fee_receipt;
            eprintln!("Initial payment: {}", fee.total_allocated_fee_payments());
            eprintln!("Fee: {}", fee.total_fees_charged());
            eprintln!("Paid: {}", fee.total_fees_paid());
            eprintln!("Refund: {}", fee.total_refunded());
            eprintln!("Unpaid: {}", fee.unpaid_debt());
            for (source, amount) in fee.fee_breakdown().iter() {
                eprintln!("- {:?} {}", source, amount);
            }
        }

        let timer = Instant::now();
        eprintln!("Finished Transaction \"{}\" in {:.2?}", tx_id, timer.elapsed());
        eprintln!();

        Ok(result)
    }

    /// Executes a transaction and commits state changes only if the transaction is accepted.
    /// Does not panic on rejection — returns the result in all cases.
    #[track_caller]
    pub fn execute_and_commit_on_success(
        &mut self,
        transaction: Transaction,
        proofs: Vec<NonFungibleAddress>,
    ) -> ExecuteResult {
        let result = self.try_execute(transaction, proofs).unwrap();
        if let Some(diff) = result.finalize.result.any_accept() {
            self.commit_diff(diff);
        }

        result
    }

    /// Returns a new [`TransactionBuilder`] configured for the local test network.
    /// Use this to construct custom transactions with multiple instructions.
    ///
    /// The builder is valid for the epoch the harness is executing in, and carries a distinct nonce
    /// so that otherwise-identical transactions get distinct ids (see [`Self::transaction_seq`]).
    pub fn transaction(&self) -> TransactionBuilder<MainIntent> {
        let seq = self.transaction_seq.get();
        self.transaction_seq.set(seq + 1);
        Transaction::builder(Network::LocalNet, self.current_epoch()).with_nonce(seq)
    }

    /// The epoch transactions execute in, as injected via [`Self::set_virtual_substate`]. Tests that
    /// remove the virtual substate entirely still have to build transactions, so those fall back to
    /// the genesis epoch.
    pub fn current_epoch(&self) -> Epoch {
        match self.virtual_substates.get(&VirtualSubstateId::CurrentEpoch) {
            Some(VirtualSubstate::CurrentEpoch(epoch)) => Epoch(*epoch),
            _ => Epoch(0),
        }
    }

    /// Executes a transaction. Panics if the transaction is not finalized (fee transaction fails). Does not panic if
    /// the main instructions fails (use execute_expect_success for that).
    #[track_caller]
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
    pub fn build_and_execute(
        &mut self,
        builder: TransactionBuilder<MainIntent>,
        proofs: Vec<NonFungibleAddress>,
    ) -> ExecuteResult {
        let transaction = builder.build_and_seal(&self.secret_key);
        self.execute_expect_commit(transaction, proofs)
    }

    /// Executes a transaction. Panics if the transaction fails.
    #[track_caller]
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
    #[track_caller]
    pub fn execute_expect_failure(
        &mut self,
        transaction: Transaction,
        proofs: Vec<NonFungibleAddress>,
    ) -> RejectReason {
        let result = self.try_execute(transaction, proofs).unwrap();
        result.expect_failure().clone()
    }

    /// Executes instructions (with no fee instructions) and commits the state diff on success.
    /// Returns an error if the transaction is rejected.
    #[track_caller]
    pub fn execute_and_commit(
        &mut self,
        instructions: Vec<Instruction>,
        proofs: Vec<NonFungibleAddress>,
    ) -> anyhow::Result<ExecuteResult> {
        self.execute_and_commit_with_fees(vec![], instructions, proofs)
    }

    /// Executes instructions with explicit fee instructions and commits the state diff on success.
    /// Returns an error if the fee transaction is rejected or the main transaction fails.
    #[track_caller]
    pub fn execute_and_commit_with_fees(
        &mut self,
        fee_instructions: Vec<Instruction>,
        instructions: Vec<Instruction>,
        proofs: Vec<NonFungibleAddress>,
    ) -> anyhow::Result<ExecuteResult> {
        let result = self.try_execute_instructions(fee_instructions, instructions, proofs)?;
        let diff = result.finalize.result.any_accept().ok_or_else(|| {
            anyhow!(
                "Transaction was rejected: {}",
                result.finalize.result.fee_reject().unwrap()
            )
        })?;

        // It is convenient to commit the state back to the staged state store in tests.
        self.commit_diff(diff);

        if let Some(reason) = result.finalize.any_reject() {
            return Err(anyhow!("Transaction failed: {}", reason));
        }

        Ok(result)
    }

    /// Parses and executes a transaction manifest string, automatically importing all registered
    /// templates. Template names are available as identifiers in the manifest without explicit
    /// `use` statements.
    ///
    /// `variables` provides named values that can be referenced in the manifest (e.g. component
    /// addresses, amounts).
    ///
    /// Returns an error if parsing fails, the transaction is rejected, or execution fails.
    #[track_caller]
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
            .filter(|(name, _)| *name != "Account")
            .map(|(name, addr)| format!("use template_{} as {};", addr, name))
            .collect::<Vec<_>>()
            .join("\n");
        let manifest = format!("{} fn main() {{ {} }}", template_imports, manifest);
        let instructions = parse_manifest(
            &manifest,
            variables.into_iter().map(|(a, b)| (a.to_string(), b)).collect(),
            Default::default(),
            Default::default(),
        )?;
        self.execute_and_commit(instructions.instructions, proofs)
    }

    /// Prints all substates in the current state store to stderr for debugging.
    pub fn print_state(&self) {
        for (k, v) in self.state_store.iter() {
            eprintln!("[{}]: {:?}", k, v.substate_value());
        }
    }
}
