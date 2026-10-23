//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Measures verifying a transaction's authorization signatures one at a time against verifying them
//! as one batch, as a function of the number of signatures.
//!
//! Every authorization signature over a transaction signs the same message, so
//! `TransactionSignature::verify_all_against_message` folds the set into a single multiscalar
//! multiplication. This is what sizes `MAX_SIGNATURES_PER_TRANSACTION`: the cap exists because every
//! node verifies these before any fee is charged, so the cost per signature is what a ceiling has to
//! be set against.
//!
//! Signatures are made over a synthetic message rather than a built transaction: deriving the
//! message hashes the whole body and is shared by both paths, so including it would measure the same
//! work twice and dilute what is being compared.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use ootle_byte_type::ToByteType;
use tari_crypto::{
    keys::{PublicKey, SecretKey},
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    tari_utilities::ByteArray,
};
use tari_ootle_common_types::signature::{TRANSACTION_SIGNING_CONTEXT, components};
use tari_ootle_transaction::{TransactionSealSignature, TransactionSignature, verify_sealed_batch};

/// `n` valid signatures over `message`, each from a distinct key — the shape a multi-input stealth
/// spend or a coinjoin produces, where every input contributes its own one-time key.
fn signatures(n: usize, message: &[u8; 64]) -> Vec<TransactionSignature> {
    let mut rng = rand::rng();
    (0..n)
        .map(|_| {
            let secret = RistrettoSecretKey::random(&mut rng);
            let public_key = RistrettoPublicKey::from_secret_key(&secret);
            TransactionSignature::new(
                public_key.to_byte_type(),
                TRANSACTION_SIGNING_CONTEXT.sign(&secret, message),
            )
        })
        .collect()
}

/// The pre-batch verification path: one double-base multiplication per signature, against the same
/// already-derived message.
fn verify_individually(signatures: &[TransactionSignature], message: [u8; 64]) -> bool {
    signatures.iter().all(|sig| sig.verify_message(message))
}

/// Decoding alone: the key decompression and signature parsing every term needs before any equation
/// can be evaluated, and the floor under both verification paths.
///
/// A signature reaches a node as bytes, so this cost is unavoidable and cannot be amortised across a
/// batch — which is what separates the speed-up achievable here from one measured over keys and
/// nonces that are already decompressed (as `tari_crypto`'s own batch benchmark is).
fn decode_only(signatures: &[TransactionSignature]) {
    for sig in signatures {
        let public_key = RistrettoPublicKey::from_canonical_bytes(sig.public_key().as_bytes()).unwrap();
        let signature = components(sig.signature()).unwrap();
        black_box((public_key, signature));
    }
}

/// A seal over `seal_message`, from a key of its own.
fn seal(seal_message: &[u8; 64]) -> TransactionSealSignature {
    let mut rng = rand::rng();
    let secret = RistrettoSecretKey::random(&mut rng);
    let public_key = RistrettoPublicKey::from_secret_key(&secret);
    TransactionSealSignature::new(
        public_key.to_byte_type(),
        TRANSACTION_SIGNING_CONTEXT.sign(&secret, seal_message),
    )
}

fn bench(c: &mut Criterion) {
    let message = [7u8; 64];
    let mut g = c.benchmark_group("transaction_signature_verification");

    for n in [1usize, 2, 4, 8, 16, 32, 64, 128, 256, 1024] {
        let sigs = signatures(n, &message);

        g.bench_with_input(BenchmarkId::new("individual", n), &n, |b, _| {
            b.iter(|| assert!(black_box(verify_individually(black_box(&sigs), message))))
        });

        g.bench_with_input(BenchmarkId::new("decode", n), &n, |b, _| {
            b.iter(|| decode_only(black_box(&sigs)))
        });

        g.bench_with_input(BenchmarkId::new("batch", n), &n, |b, _| {
            b.iter(|| {
                assert!(black_box(TransactionSignature::verify_all_against_message(
                    black_box(&sigs),
                    message
                )))
            })
        });
    }

    // What a node pays to refuse a set, which a rejected transaction's sender does not cover. The
    // invalid signature goes last, the costliest position for the per-signature path this replaces
    // and so the one an attacker would have chosen against it.
    for n in [1usize, 16, 256] {
        let mut sigs = signatures(n, &message);
        let last = sigs.len() - 1;
        sigs[last] = signatures(1, &[8u8; 64]).remove(0);

        g.bench_with_input(BenchmarkId::new("rejected_individual", n), &n, |b, _| {
            b.iter(|| assert!(!black_box(verify_individually(black_box(&sigs), message))))
        });

        g.bench_with_input(BenchmarkId::new("rejected_batch", n), &n, |b, _| {
            b.iter(|| {
                assert!(!black_box(TransactionSignature::verify_all_against_message(
                    black_box(&sigs),
                    message
                )))
            })
        });
    }

    // The whole signature set of a sealed transaction. The seal signs a different message from the
    // authorizations, so this measures what folding it into the same multiplication is worth over
    // verifying it on its own and batching the rest.
    for n in [1usize, 4, 16] {
        let authorization_message = [7u8; 64];
        let seal_message = [9u8; 64];
        let sigs = signatures(n, &authorization_message);
        let seal = seal(&seal_message);

        g.bench_with_input(BenchmarkId::new("sealed_seal_apart", n), &n, |b, _| {
            b.iter(|| {
                assert!(black_box(seal.verify_message(seal_message)));
                assert!(black_box(TransactionSignature::verify_all_against_message(
                    black_box(&sigs),
                    authorization_message
                )))
            })
        });

        g.bench_with_input(BenchmarkId::new("sealed_one_batch", n), &n, |b, _| {
            b.iter(|| {
                assert!(black_box(verify_sealed_batch(
                    black_box(&seal),
                    seal_message,
                    black_box(&sigs),
                    authorization_message,
                )))
            })
        });
    }

    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
