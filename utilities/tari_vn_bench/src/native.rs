//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Native cryptographic verification, which runs outside the WASM meter.
//!
//! Two paths matter to a validator's ability to keep up, and neither is covered by the execution
//! phase:
//!
//! * **Signature verification** is the mempool's admission cost. Every transaction that arrives over gossip is verified
//!   before anything else happens to it, whether or not it is ever sequenced, so this is the one cost a node pays for
//!   traffic it does not benefit from — and the one an attacker can raise for free.
//! * **Stealth transfer verification** is real CPU inside block execution. It is priced into the block execution- point
//!   budget by wall-clock equivalence against the WASM rate, so a host whose curve arithmetic is disproportionately
//!   slow relative to its WASM throughput carries a cost the points budget does not see. This phase is what makes that
//!   visible.

use std::{hint::black_box, time::Instant};

use serde::{Deserialize, Serialize};
use tari_crypto::ristretto::{RistrettoPublicKey, RistrettoSecretKey};
use tari_engine_types::stealth::validate_transfer;
use tari_ootle_transaction::{Transaction, args};
use tari_template_lib::types::{ComponentAddress, constants::TARI_TOKEN};
use tari_template_test_tooling::{TemplateTest, support::stealth::generate_transfer_data, wallet_crypto::MaskAndValue};

use crate::stats::Sample;

/// Trials per verification. Both operations are sub-millisecond, so a large sample costs little and
/// buys a trustworthy minimum — at small counts the per-operation figure was seen to move by
/// multiples between runs on a lightly loaded machine.
const TRIALS: usize = 500;
const TRIALS_QUICK: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeMeasurement {
    /// Time to verify every signature on one canonical transfer: the seal signature plus the
    /// signer's. This is what mempool admission pays per gossiped transaction.
    pub signature_verify: Sample,
    pub signature_verifies_per_sec: f64,
    /// Time to verify one stealth transfer statement with a single input and two outputs — the
    /// canonical shape (recipient plus change) the native point prices are fitted around.
    pub stealth_transfer_verify: Sample,
    pub stealth_verifies_per_sec: f64,
}

pub fn measure(quick: bool) -> anyhow::Result<NativeMeasurement> {
    let trials = if quick { TRIALS_QUICK } else { TRIALS };
    let signature_verify = measure_signature_verify(trials)?;
    let stealth_transfer_verify = measure_stealth_verify(trials);

    Ok(NativeMeasurement {
        signature_verifies_per_sec: 1000.0 / signature_verify.min_ms,
        signature_verify,
        stealth_verifies_per_sec: 1000.0 / stealth_transfer_verify.min_ms,
        stealth_transfer_verify,
    })
}

/// Times `Transaction::verify_all_signatures` on the same transfer shape the execution phase
/// builds, so the two phases describe the same transaction from ingress to execution.
fn measure_signature_verify(trials: usize) -> anyhow::Result<Sample> {
    // Only the transaction is needed, but building a realistic one means building the accounts it
    // refers to; the harness is the cheapest way to get them.
    let mut test = TemplateTest::new_builtin_only();
    let (sender, _, key) = test.create_funded_account();
    let (receiver, _, _) = test.create_empty_account();
    let transaction = build_transfer(&test, sender, receiver, &key);

    if !transaction.verify_all_signatures() {
        anyhow::bail!("benchmark transaction failed its own signature check; the harness built it wrong");
    }

    let mut ms = Vec::with_capacity(trials);
    for _ in 0..trials {
        let started = Instant::now();
        black_box(black_box(&transaction).verify_all_signatures());
        ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(Sample::from_millis(ms))
}

fn build_transfer(
    test: &TemplateTest,
    sender: ComponentAddress,
    receiver: ComponentAddress,
    key: &RistrettoSecretKey,
) -> Transaction {
    test.transaction()
        .with_unversioned_inputs([sender, receiver])
        .call_method(sender, "withdraw", args![TARI_TOKEN, 1])
        .put_last_instruction_output_on_workspace("transferred")
        .call_method(receiver, "deposit", args![Workspace("transferred")])
        .build_and_seal(key)
}

/// Times `validate_transfer` on a one-input, two-output statement with no view key — a transfer to
/// a recipient with change, on a resource that carries no viewable-balance surcharge.
fn measure_stealth_verify(trials: usize) -> Sample {
    let inputs = vec![MaskAndValue {
        mask: RistrettoSecretKey::from(1u64),
        value: 1_000,
    }];
    // Inputs and outputs must balance, so the two outputs sum to the single input.
    let data = generate_transfer_data(inputs, 0u64, vec![600u64, 400u64], 0u64);
    let statement = data.statement;
    let no_view_key: Option<&RistrettoPublicKey> = None;

    validate_transfer(&statement, no_view_key).expect("generated statement must verify");

    let mut ms = Vec::with_capacity(trials);
    for _ in 0..trials {
        let started = Instant::now();
        black_box(validate_transfer(black_box(&statement), no_view_key)).expect("verifies");
        ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    Sample::from_millis(ms)
}
