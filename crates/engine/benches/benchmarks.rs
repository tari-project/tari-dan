//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod common;

use std::{hint::black_box, iter, sync::Arc};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use ootle_network::Network;
use tari_engine::{
    executables::{Executable, Instructions, WeightedExecutable},
    fees::WasmMeteringRate,
    runtime::AuthParams,
    state_store::memory::ReadOnlyMemoryStateStore,
    transaction::TransactionProcessor,
};
use tari_engine_types::{
    fees::ExhaustBurnRate,
    virtual_substate::{VirtualSubstate, VirtualSubstateId, VirtualSubstates},
};
use tari_ootle_common_types::InputDeclaration;
use tari_ootle_transaction::{Instruction, TransactionId, TransactionWeight, call_args};
use tari_template_lib::types::{Hash32, constants::XTR_FAUCET_CLAIM_RESOURCE_ADDRESS, crypto::RistrettoPublicKeyBytes};
use tari_template_test_tooling::{Package, mocks::AlwaysPassesProofVerifier};

use crate::common::{FAUCET_COMPONENT_ADDRESS, FAUCET_VAULT_ID, setup_store};

type BenchTxProcessor = TransactionProcessor<ReadOnlyMemoryStateStore, Package>;

pub struct CreateAndFundAccountExecutable;

impl Executable for CreateAndFundAccountExecutable {
    fn to_id(&self) -> TransactionId {
        TransactionId::default()
    }

    fn calculate_intent_commitment(&self) -> Hash32 {
        Hash32::default()
    }

    fn all_inputs_iter(&self) -> impl Iterator<Item = InputDeclaration> + '_ {
        [
            InputDeclaration::write(FAUCET_COMPONENT_ADDRESS),
            InputDeclaration::write(FAUCET_VAULT_ID),
            InputDeclaration::read(XTR_FAUCET_CLAIM_RESOURCE_ADDRESS),
        ]
        .into_iter()
    }

    fn signers_iter(&self) -> impl Iterator<Item = &RistrettoPublicKeyBytes> {
        static SIGNER: RistrettoPublicKeyBytes = RistrettoPublicKeyBytes::zero();
        iter::once(&SIGNER)
    }

    fn into_instructions(self) -> Instructions {
        Instructions {
            fee: vec![
                Instruction::CreateAccount {
                    owner_public_key: Default::default(),
                    owner_rule: None,
                    access_rules: None,
                    bucket_workspace_id: None,
                },
                Instruction::PutLastInstructionOutputOnWorkspace { key: 0 },
                Instruction::CallMethod {
                    call: FAUCET_COMPONENT_ADDRESS.into(),
                    method: "take".try_into().unwrap(),
                    args: call_args![Workspace(0)],
                },
            ],
            main: vec![],
            blobs: Default::default(),
        }
    }
}

impl WeightedExecutable for CreateAndFundAccountExecutable {
    fn calculate_weight(&self) -> TransactionWeight {
        TransactionWeight::new(100)
    }
}

pub struct SharedState {
    package: Arc<Package>,
    virtual_substates: VirtualSubstates,
    state_store: ReadOnlyMemoryStateStore,
    claim_burn_proof_verifier: Arc<AlwaysPassesProofVerifier>,
}

fn setup(shared: &SharedState) -> BenchTxProcessor {
    let auth_params = AuthParams::default();
    let modules = Default::default();
    BenchTxProcessor::new(
        shared.package.clone(),
        shared.state_store.clone(),
        auth_params,
        shared.virtual_substates.clone(),
        modules,
        shared.claim_burn_proof_verifier.clone(),
        WasmMeteringRate::unmetered(),
        ExhaustBurnRate::new(0),
        0,
        Network::LocalNet,
        false,
    )
}

fn criterion_benchmark(c: &mut Criterion) {
    let mut package = Package::builder();
    package.add_all_builtin_templates();
    let state_store = setup_store();
    let virtual_substates = VirtualSubstates::from_iter([
        (VirtualSubstateId::CurrentEpoch, VirtualSubstate::CurrentEpoch(1)),
        (
            VirtualSubstateId::CurrentEpochHash,
            VirtualSubstate::CurrentEpochHash([0u8; 32].into()),
        ),
    ]);
    let claim_burn_proof_verifier = Arc::new(AlwaysPassesProofVerifier);
    let shared = SharedState {
        package: Arc::new(package.build()),
        virtual_substates,
        state_store: state_store.into_read_only(),
        claim_burn_proof_verifier,
    };

    let id = BenchmarkId::new("create_and_fund_account", "");

    c.bench_with_input(id, &shared, |b, shared| {
        b.iter_batched(
            || setup(shared),
            |processor| {
                let res = black_box(processor).execute(CreateAndFundAccountExecutable).unwrap();
                res.expect_success();
                // Return to avoid the compiler optimising anything out, since criterion will black_box the return. Not
                // 100% sure if it makes a difference, but just in case
                res
            },
            criterion::BatchSize::SmallInput,
        )
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
