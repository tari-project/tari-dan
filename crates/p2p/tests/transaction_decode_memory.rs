//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Heap cost of decoding a peer-supplied transaction.

mod support;

use std::{hint::black_box, sync::atomic::Ordering};

use prost::Message;
use support::{OUTSTANDING, PEAK, SERIAL};
use tari_consensus::messages::MissingTransactionsResponse;
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_ootle_p2p::proto;
use tari_ootle_transaction::{
    Epoch,
    Instruction,
    MAX_INSTRUCTION_ARGS,
    MAX_TRANSACTION_ACCESS_RULE_NODES,
    MAX_TRANSACTION_INSTRUCTION_ARGS,
    MAX_TRANSACTION_INSTRUCTIONS,
    Transaction,
    args::InstructionArg,
};
use tari_swarm::messaging::MAX_MESSAGE_SIZE;
use tari_template_lib::types::{
    FunctionName,
    Hash32,
    access_rules::{AccessRule, ComponentAccessRules, RestrictedAccessRule},
    crypto::RistrettoPublicKeyBytes,
};

/// `ConsensusConstants::max_transaction_size_bytes`: the most bytes a transaction may encode to and
/// still be relayed.
const MAX_TRANSACTION_SIZE_BYTES: usize = 1_310_720;

/// The most heap decoding one transaction may take, whatever it carries: both instruction lists at their
/// limit, plus room for the lists' growth while they decode.
const MAX_DECODE_HEAP: usize = 2 * MAX_TRANSACTION_INSTRUCTIONS * size_of::<Instruction>() * 3 / 2;

/// The wire form of a sealed transaction whose fee and main instruction lists carry the given number of
/// copies of the cheapest instruction to encode.
fn wire_with_drop_instructions(fee_instructions: usize, instructions: usize) -> proto::transaction::Transaction {
    let transaction = Transaction::builder_localnet(Epoch(1))
        .with_fee_instructions((0..fee_instructions).map(|_| Instruction::DropAllProofsInWorkspace))
        .with_instructions((0..instructions).map(|_| Instruction::DropAllProofsInWorkspace))
        .build_and_seal(&RistrettoSecretKey::from(1u64));
    proto::transaction::Transaction::from(&transaction)
}

/// The most heap decoding `wire` takes at any point, over what was allocated before it started, and whether
/// the decode succeeded.
fn peak_decode_heap(wire: proto::transaction::Transaction) -> (usize, bool) {
    let baseline = OUTSTANDING.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    let decoded = Transaction::try_from(wire);
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(baseline);
    let is_ok = decoded.is_ok();
    drop(black_box(decoded));
    (peak, is_ok)
}

/// Every validator decodes a relayed transaction before it can judge it, so the heap one decode takes must
/// be bounded by a constant rather than by how many instructions fit the byte cap. The costliest transaction
/// that still decodes fills both instruction lists with the cheapest instruction to encode.
#[test]
fn the_costliest_transaction_that_decodes_stays_within_a_fixed_heap_bound() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let wire = wire_with_drop_instructions(MAX_TRANSACTION_INSTRUCTIONS, MAX_TRANSACTION_INSTRUCTIONS);
    let wire_len = wire.bor_encoded.len();

    let (peak, is_ok) = peak_decode_heap(wire);

    assert!(is_ok, "a transaction at both instruction limits decodes");
    assert!(
        peak <= MAX_DECODE_HEAP,
        "decoding {wire_len} bytes peaked at {peak} bytes of heap, over the {MAX_DECODE_HEAP} byte bound"
    );
}

/// Call arguments and access rules at their limits in both lists, besides the instructions, still decode within
/// the same bound.
#[test]
fn a_transaction_at_every_instruction_limit_stays_within_the_same_bound() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let list = || {
        let rules = (0..MAX_TRANSACTION_ACCESS_RULE_NODES).fold(ComponentAccessRules::new(), |rules, i| {
            rules.method(format!("{i:x}"), AccessRule::AllowAll)
        });
        let calls = MAX_TRANSACTION_INSTRUCTION_ARGS / MAX_INSTRUCTION_ARGS;
        (0..calls)
            .map(|_| call_function(MAX_INSTRUCTION_ARGS))
            .chain([create_account(rules)])
            .chain((calls + 1..MAX_TRANSACTION_INSTRUCTIONS).map(|_| Instruction::DropAllProofsInWorkspace))
            .collect::<Vec<_>>()
    };
    let transaction = Transaction::builder_localnet(Epoch(1))
        .with_fee_instructions(list())
        .with_instructions(list())
        .build_and_seal(&RistrettoSecretKey::from(1u64));
    let wire = proto::transaction::Transaction::from(&transaction);
    let wire_len = wire.bor_encoded.len();

    let (peak, is_ok) = peak_decode_heap(wire);

    assert!(is_ok, "a transaction at every instruction limit decodes");
    assert!(
        peak <= MAX_DECODE_HEAP,
        "decoding {wire_len} bytes peaked at {peak} bytes of heap, over the {MAX_DECODE_HEAP} byte bound"
    );
}

/// Instructions that fit the byte cap but exceed the instruction limit are refused for less than the
/// costliest transaction that decodes.
#[test]
fn a_max_size_transaction_over_the_instruction_limit_is_refused_within_the_same_bound() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let empty = wire_with_drop_instructions(0, 0).bor_encoded.len();
    let per_instruction = (wire_with_drop_instructions(0, 1000).bor_encoded.len() - empty).div_ceil(1000);
    let wire = wire_with_drop_instructions(0, (MAX_TRANSACTION_SIZE_BYTES - empty) / per_instruction);
    let wire_len = wire.bor_encoded.len();
    assert!(wire_len <= MAX_TRANSACTION_SIZE_BYTES);

    let (peak, is_ok) = peak_decode_heap(wire);

    assert!(!is_ok, "a transaction over the instruction limit does not decode");
    assert!(
        peak <= MAX_DECODE_HEAP,
        "refusing {wire_len} bytes peaked at {peak} bytes of heap, over the {MAX_DECODE_HEAP} byte bound"
    );
}

/// The wire form of a sealed transaction whose main instruction list is `instructions(n)`, for the largest `n`
/// up to `max_n` that keeps the transaction within the byte cap.
fn largest_within_byte_cap(
    max_n: usize,
    instructions: impl Fn(usize) -> Vec<Instruction>,
) -> proto::transaction::Transaction {
    let wire = |n: usize| {
        let transaction = Transaction::builder_localnet(Epoch(1))
            .with_instructions(instructions(n))
            .build_and_seal(&RistrettoSecretKey::from(1u64));
        proto::transaction::Transaction::from(&transaction)
    };
    let one = wire(1).bor_encoded.len();
    let per_unit = (wire(1001).bor_encoded.len() - one) as f64 / 1000.0;
    let mut n = (1 + ((MAX_TRANSACTION_SIZE_BYTES - one) as f64 / per_unit) as usize).min(max_n);
    loop {
        let candidate = wire(n);
        let len = candidate.bor_encoded.len();
        if len <= MAX_TRANSACTION_SIZE_BYTES {
            return candidate;
        }
        n = n * MAX_TRANSACTION_SIZE_BYTES / len - 1;
    }
}

fn create_account(access_rules: ComponentAccessRules) -> Instruction {
    Instruction::CreateAccount {
        owner_public_key: RistrettoPublicKeyBytes::from_bytes(&[1; 32]).unwrap(),
        owner_rule: None,
        access_rules: Some(access_rules),
        bucket_workspace_id: None,
    }
}

/// An access rule requiring any of `width` empty alternatives: the cheapest rule node to encode.
fn wide_rule(width: usize) -> AccessRule {
    AccessRule::Restricted(RestrictedAccessRule::AnyOf(
        (0..width).map(|_| RestrictedAccessRule::AnyOf(Box::new([]))).collect(),
    ))
}

fn call_function(num_args: usize) -> Instruction {
    Instruction::CallFunction {
        address: Hash32::from_array([1; 32]),
        function: FunctionName::default(),
        args: (0..num_args).map(|_| InstructionArg::workspace(0, None)).collect(),
    }
}

/// Asserts that `wire` is refused, and that refusing it takes no more heap than the costliest transaction that
/// decodes.
fn assert_refused_within_bound(wire: proto::transaction::Transaction) {
    let wire_len = wire.bor_encoded.len();
    let (peak, is_ok) = peak_decode_heap(wire);
    assert!(
        peak <= MAX_DECODE_HEAP,
        "decoding {wire_len} bytes peaked at {peak} bytes of heap, over the {MAX_DECODE_HEAP} byte bound"
    );
    assert!(
        !is_ok,
        "a transaction of {wire_len} bytes over the decode limits decodes"
    );
}

/// One access rule as wide as the byte cap allows.
#[test]
fn a_max_size_access_rule_is_refused_within_the_same_bound() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let wire = largest_within_byte_cap(usize::MAX, |n| {
        vec![create_account(ComponentAccessRules::new().method("m", wide_rule(n)))]
    });
    assert_refused_within_bound(wire);
}

/// One component's method rules as numerous as the byte cap allows.
#[test]
fn a_max_size_method_access_map_is_refused_within_the_same_bound() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let wire = largest_within_byte_cap(usize::MAX, |n| {
        let rules = (0..n).fold(ComponentAccessRules::new(), |rules, i| {
            rules.method(format!("{i:x}"), AccessRule::AllowAll)
        });
        vec![create_account(rules)]
    });
    assert_refused_within_bound(wire);
}

/// Narrow access rules on as many instructions as the byte cap allows.
#[test]
fn access_rules_spread_over_max_size_instructions_are_refused_within_the_same_bound() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let wire = largest_within_byte_cap(MAX_TRANSACTION_INSTRUCTIONS, |n| {
        (0..n)
            .map(|_| create_account(ComponentAccessRules::new().method("m", wide_rule(32))))
            .collect()
    });
    assert_refused_within_bound(wire);
}

/// One call with as many arguments as the byte cap allows.
#[test]
fn a_max_size_argument_list_is_refused_within_the_same_bound() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let wire = largest_within_byte_cap(usize::MAX, |n| vec![call_function(n)]);
    assert_refused_within_bound(wire);
}

/// Calls carrying the most arguments a function takes, on as many instructions as the byte cap allows.
#[test]
fn arguments_spread_over_max_size_instructions_are_refused_within_the_same_bound() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let wire = largest_within_byte_cap(MAX_TRANSACTION_INSTRUCTIONS, |n| {
        (0..n).map(|_| call_function(32)).collect()
    });
    assert_refused_within_bound(wire);
}

/// A missing-transactions response arrives from a peer before it can be matched to a request this node made,
/// so building it from the wire must cost no more than the bytes it arrived as, however many costly
/// transactions the frame packs.
#[test]
fn a_frame_of_costly_transactions_builds_a_response_within_its_wire_size() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let transaction = wire_with_drop_instructions(MAX_TRANSACTION_INSTRUCTIONS, MAX_TRANSACTION_INSTRUCTIONS);
    let mut response = proto::consensus::MissingTransactionsResponse {
        request_id: 1,
        epoch: 1,
        block_id: vec![0; 32],
        transactions: vec![],
    };
    while response.encoded_len() + transaction.encoded_len() + 8 <= MAX_MESSAGE_SIZE {
        response.transactions.push(transaction.clone());
    }
    let num_transactions = response.transactions.len();
    let wire_len = response.encoded_len();

    let baseline = OUTSTANDING.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    let built = MissingTransactionsResponse::try_from(response);
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(baseline);
    let is_ok = built.is_ok();
    drop(black_box(built));

    assert!(
        is_ok,
        "a response of {num_transactions} transactions is accepted from the wire"
    );
    assert!(
        peak <= wire_len,
        "building a response of {num_transactions} transactions from {wire_len} bytes peaked at {peak} bytes of heap"
    );
}
