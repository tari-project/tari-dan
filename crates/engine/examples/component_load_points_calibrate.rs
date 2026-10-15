//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Prices the host-side work of loading a component for a call in metering points.
//!
//! Every call on a component has the engine walk the component's state outside the Wasmer meter: to
//! count its items, to index the substates it holds and to bring them into the new frame's scope.
//! Each shape below is written into a component's state directly in the store, filling
//! `max_internal_call_size` with the cheapest-to-encode item of its kind, and a template calls a
//! method on that component that leaves the state unread. The time each extra call adds over the
//! same call on a component holding nothing is set against what the shape adds. The shapes holding
//! no references bound `PER_COMPONENT_STATE_ITEM`; those of substate addresses, each decoded and
//! inserted into the scope, bound `PER_COMPONENT_STATE_REFERENCE`. The single byte string shows the
//! load does nothing per byte.
//!
//! The time is converted to points at the same WASM points-per-millisecond rate
//! `native_points_calibrate` derives, and by the same method, so the figures are comparable.
//!
//! Run with `--release`; a debug build measures nothing useful.

use std::time::Instant;

use tari_engine::{fees::FeeTable, state_store::StateWriter};
use tari_engine_types::{
    indexed_value::IndexedWellKnownTypes,
    limits::{self, NativeExecutionPoints},
    substate::{Substate, SubstateId},
};
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_lib::types::{
    BinaryTag,
    ComponentAddress,
    NonFungibleAddress,
    NonFungibleId,
    ObjectKey,
    ResourceAddress,
    bytes::Bytes,
};
use tari_template_test_tooling::TemplateTest;

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");
const METERING_BENCH: &str = "tests/templates/metering_bench";
const LIMITS: &str = "tests/templates/limits";
const LIMITS_TEMPLATE: &str = "PushItToTheLimit";

/// Executions timed per round or call count; the fastest is kept.
const ENGINE_TRIALS: usize = 7;
/// Round counts for the WASM rate fit.
const R1: u64 = 5_000;
const R2: u64 = 10_000;
/// Method call counts for the per-call fit. A call on the largest shape is charged ~2e8 points, so
/// the larger count stays under `MAX_NATIVE_POINTS_PER_TRANSACTION`.
const C1: u32 = 1;
const C2: u32 = 9;
const MAX_FEE: u64 = 60_000_000;

const MAX: usize = limits::ENGINE_LIMITS.max_internal_call_size;

fn array_of(len: usize, item: impl Fn(usize) -> tari_bor::Value) -> tari_bor::Value {
    tari_bor::Value::Array((0..len).map(item).collect())
}

/// A tagged 32-byte key, distinct for each `i`, as a substate address of kind `tag` encodes.
fn tagged_key(tag: BinaryTag, i: usize) -> tari_bor::Value {
    let mut key = [0u8; 32];
    key[..8].copy_from_slice(&(i as u64).to_be_bytes());
    tari_bor::Value::Tag(tag.as_u64(), Box::new(tari_bor::Value::Bytes(key.to_vec())))
}

/// Values filling `max_internal_call_size`, one per item shape.
fn value_shapes() -> Vec<(&'static str, tari_bor::Value)> {
    use tari_bor::Value;
    let n = MAX - 5;
    // A tag, a byte string head and 32 bytes.
    let keyed = n / 36;
    vec![
        ("[0, 0, ..]", array_of(n, |_| Value::Integer(0))),
        ("[null, null, ..]", array_of(n, |_| Value::Null)),
        ("[[], [], ..]", array_of(n, |_| Value::Array(vec![]))),
        ("[{}, {}, ..]", array_of(n, |_| Value::Map(vec![]))),
        (
            "[[[0]], [[0]], ..]",
            array_of(n / 3, |_| Value::Array(vec![Value::Array(vec![Value::Integer(0)])])),
        ),
        (
            "{0: 0, ..}",
            Value::Map((0..n / 2).map(|_| (Value::Integer(0), Value::Integer(0))).collect()),
        ),
        (
            "[0(0), 0(0), ..]",
            array_of(n / 2, |_| Value::Tag(0, Box::new(Value::Integer(0)))),
        ),
        (
            "[component, ..]",
            array_of(keyed, |i| tagged_key(BinaryTag::ComponentAddress, i)),
        ),
        (
            "[resource, ..]",
            array_of(keyed, |i| tagged_key(BinaryTag::ResourceAddress, i)),
        ),
        ("[vault, ..]", array_of(keyed, |i| tagged_key(BinaryTag::VaultId, i))),
        ("[nft, ..]", array_of(n / non_fungible_len(), non_fungible)),
        ("h'abab..'", Value::Bytes(vec![0xab; n])),
    ]
}

/// A non-fungible address with an integer id, distinct for each `i`.
fn non_fungible(i: usize) -> tari_bor::Value {
    let mut key = [0u8; ObjectKey::LENGTH];
    key[..8].copy_from_slice(&(i as u64).to_be_bytes());
    let address = NonFungibleAddress::new(
        ResourceAddress::new(ObjectKey::from_array(key)),
        NonFungibleId::from_u64(i as u64),
    );
    tari_bor::to_value(&address).unwrap()
}

fn non_fungible_len() -> usize {
    tari_bor::encode(&non_fungible(u32::MAX as usize)).unwrap().len()
}

/// A test whose fee table charges one fee unit per metered WASM point.
fn metered_test(templates: &[&str]) -> TemplateTest {
    let mut test = TemplateTest::new(CRATE_PATH, templates);
    let mut fee_table = FeeTable::zero_rated();
    fee_table.per_wasm_point_cost = 1;
    fee_table.wasm_points_cost_divisor = 1;
    test.set_fee_table(fee_table);
    test.enable_fees();
    test
}

/// The points-per-millisecond rate, derived exactly as `native_points_calibrate` derives it: two
/// round counts of the same metered loop, fitted on the marginal points over the marginal time.
fn wasm_rate_points_per_ms() -> f64 {
    let mut test = metered_test(&[METERING_BENCH]);
    let bench = test.get_template_address("MeteringBench");
    let (account, owner, key) = test.create_funded_account();

    let mut run = |rounds: u64| -> (u64, f64) {
        let execute = |test: &mut TemplateTest| -> (u64, f64) {
            let tx = Transaction::builder_localnet(Epoch(1))
                .pay_fee_from_component(account, MAX_FEE)
                .call_function(bench, "bench_div_u64", args![rounds])
                .build_and_seal(&key);
            let start = Instant::now();
            let result = test.execute_expect_success(tx, vec![owner.clone()]);
            let elapsed = start.elapsed().as_nanos() as f64 / 1e6;
            (result.wasm_execution_points, elapsed)
        };
        let _ = execute(&mut test);
        let mut points = 0;
        let mut ms = Vec::with_capacity(ENGINE_TRIALS);
        for _ in 0..ENGINE_TRIALS {
            let (p, t) = execute(&mut test);
            points = p;
            ms.push(t);
        }
        ms.sort_by(f64::total_cmp);
        (points, ms[0])
    };

    let (points_r1, ms_r1) = run(R1);
    let (points_r2, ms_r2) = run(R2);
    (points_r2 - points_r1) as f64 / (ms_r2 - ms_r1)
}

/// A `PushItToTheLimit` component open to calls from any frame, its `extra` field set to `extra`
/// directly in the store.
fn component_holding(test: &mut TemplateTest, extra: tari_bor::Value) -> ComponentAddress {
    let component: ComponentAddress =
        test.call_function(LIMITS_TEMPLATE, "new", args![Bytes::from(vec![0u8; 4])], vec![]);
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(component, "allow_all_methods", args![])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    let id = SubstateId::Component(component);
    let substate = test.read_only_state_store().get_substate(&id).unwrap();
    let version = substate.version();
    let mut value = substate.into_substate_value();
    let tari_bor::Value::Array(fields) = &mut value.component_mut().unwrap().body.state else {
        panic!("the component state is an array of its fields");
    };
    *fields.last_mut().unwrap() = extra;
    test.get_state_store_mut()
        .set_state(id, Substate::new(version, value))
        .unwrap();
    component
}

/// The time and native points one extra method call on `component` adds, fitted over two call
/// counts as the WASM rate is.
fn per_method_call(test: &mut TemplateTest, component: ComponentAddress) -> (f64, f64) {
    let template = test.get_template_address(LIMITS_TEMPLATE);
    let mut run = |calls: u32| -> (u64, f64) {
        let execute = |test: &mut TemplateTest| -> (u64, f64) {
            let tx = Transaction::builder_localnet(Epoch(1))
                .call_function(template, "touch_component", args![component, calls])
                .build_and_seal(test.secret_key());
            let start = Instant::now();
            let result = test.execute_expect_success(tx, vec![]);
            let elapsed = start.elapsed().as_nanos() as f64 / 1e6;
            (result.native_execution_points, elapsed)
        };
        let _ = execute(test);
        let mut native = 0;
        let mut ms = Vec::with_capacity(ENGINE_TRIALS);
        for _ in 0..ENGINE_TRIALS {
            let (n, t) = execute(test);
            native = n;
            ms.push(t);
        }
        ms.sort_by(f64::total_cmp);
        (native, ms[0])
    };

    let (native_c1, ms_c1) = run(C1);
    let (native_c2, ms_c2) = run(C2);
    let calls = f64::from(C2 - C1);
    ((ms_c2 - ms_c1) / calls, (native_c2 - native_c1) as f64 / calls)
}

fn main() {
    let rate = wasm_rate_points_per_ms();
    println!("WASM rate: {rate:.0} points/ms\n");

    // Fees stay off: the loads' native charges would outrun any fee this test could pay, and the
    // points are read from the execution itself.
    let mut test = TemplateTest::new(CRATE_PATH, [LIMITS]);
    let empty = component_holding(&mut test, tari_bor::Value::Null);
    let (empty_ms, empty_native) = per_method_call(&mut test, empty);
    println!("Method call on a component holding nothing: {empty_ms:.4} ms, {empty_native:.0} native points\n");

    println!("Component load: the item count, the index of the state's substates and the frame's scope");
    println!(
        "{:<24} {:>8} {:>8} {:>10} {:>12} {:>10}",
        "shape", "items", "refs", "ms", "points/each", "charged/ms"
    );
    for (label, extra) in value_shapes() {
        let count = extra
            .count_items_bounded(u64::MAX, IndexedWellKnownTypes::indexes_tag)
            .unwrap();
        // `extra` replaces a null, one item.
        let (items, references) = (count.items - 1, count.counted_tags);
        let component = component_holding(&mut test, extra);
        let (ms, native) = per_method_call(&mut test, component);
        let ms = ms - empty_ms;
        let points = ms * rate;
        let charged = native - empty_native;
        assert_eq!(charged as u64, limits::component_load_points(items, references));
        // A shape of references is shown per reference, net of its items' charge.
        let per_each = if references == 0 {
            points / items.max(1) as f64
        } else {
            (points - limits::component_load_points(items, 0) as f64) / references as f64
        };
        println!(
            "{label:<24} {items:>8} {references:>8} {ms:>10.3} {per_each:>12.1} {:>9.2}x",
            charged / points,
        );
    }

    println!(
        "\nPER_COMPONENT_STATE_ITEM = {}, PER_COMPONENT_STATE_REFERENCE = {}. The charged column is what they charge \
         over what was measured.",
        NativeExecutionPoints::PER_COMPONENT_STATE_ITEM,
        NativeExecutionPoints::PER_COMPONENT_STATE_REFERENCE,
    );
}
