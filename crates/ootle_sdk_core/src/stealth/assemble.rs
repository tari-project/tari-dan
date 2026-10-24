//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Stealth statement assembly — combines the outputs and inputs statements into a complete transfer
//! statement and builds the unsigned transaction. Pure and synchronous.
//!
//! The outputs and inputs paths produce the two halves of a [`StealthTransferStatement`]; this module
//! combines them, generates the balance proof, runs two pre-flight validations
//! (`validate_balance_proof_signature` + `validate_transfer`), then assembles
//! [`Instruction::StealthTransfer`](tari_ootle_transaction) into an [`UnsignedTransaction`]. The
//! result is an opaque [`StealthPartialTransaction`] carrying the unsigned tx + the signing
//! requirements the sign/seal path needs.
//!
//! ## Semantic determinism
//!
//! Neither the aggregated bulletproof nor the balance-proof Schnorr signature is byte-stable: both
//! draw a nonce from `tari_crypto`'s / `tari_ootle_wallet_crypto`'s internal RNG, with no public seam
//! to inject a deterministic nonce. `entropy.balance_proof_nonce` is therefore reserved but **not**
//! consumed here: injecting it would require a change to the shared
//! `tari_ootle_wallet_crypto::balance_proof` leaf. The deterministic path still calls **no RNG of its
//! own** — the only randomness is inside the reused crypto leaves.
//! Everything else the deterministic path produces is byte-reproducible: the inputs/outputs
//! statements (modulo `agg_range_proof`), the instruction sequence, the bucket workspace ids, the
//! epoch window and dry-run flag.

use std::collections::HashMap;

use tari_crypto::{ristretto::RistrettoSecretKey, tari_utilities::ByteArray};
use tari_engine_types::stealth::validate_transfer;
use tari_ootle_transaction::{TransactionBuilder, UnsignedTransaction, args};
use tari_ootle_wallet_crypto::balance_proof::{
    generate_stealth_balance_proof_signature,
    validate_balance_proof_signature,
};
use tari_template_lib_types::{
    Amount,
    ComponentAddress,
    ResourceAddress,
    crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes},
    stealth::{StealthInput, StealthInputsStatement, StealthOutputsStatement, StealthTransferStatement},
};

use crate::{
    builder::bucket_label,
    inputs::{FetchedSubstate, PartialTransaction, Resolution, WantList, apply_fetched_substates_with_secrets},
    stealth::{
        inputs::spend_secrets_map,
        outputs::build_stealth_outputs_statement_from_entropy,
        partial::{StealthPartialTransaction, StealthSignatureRequirementsState},
        sign_seal::{SpendSecrets, StealthKeys, seal_public_key},
    },
    types::{
        bytes::{BuildSeed, SecretKeyBytes},
        error::OotleSdkError,
        network::Network,
        stealth::{StealthEntropy, StealthTransferIntent},
    },
};

/// Parses a boundary secret scalar into the internal [`RistrettoSecretKey`].
fn parse_mask(bytes: &SecretKeyBytes) -> Result<RistrettoSecretKey, OotleSdkError> {
    RistrettoSecretKey::from_canonical_bytes(bytes.as_bytes())
        .map_err(|e| OotleSdkError::Key(format!("invalid aggregate mask: {e}")))
}

/// Builds the [`StealthInputsStatement`] from the intent's stealth inputs + revealed input amount.
///
/// One [`StealthInput`] per spent UTXO (its on-chain commitment) plus the total revealed input
/// amount. The commitments come straight from the intent (the input resolver has already validated
/// each one exists on-chain and is spendable).
fn build_inputs_statement(intent: &StealthTransferIntent) -> Result<StealthInputsStatement, OotleSdkError> {
    let inputs = intent
        .inputs
        .iter()
        .map(|i| {
            let commitment = PedersenCommitmentBytes::from_bytes(i.commitment.as_bytes())
                .map_err(|e| OotleSdkError::Key(format!("invalid input commitment: {e}")))?;
            Ok(StealthInput::new(commitment))
        })
        .collect::<Result<Vec<_>, OotleSdkError>>()?;

    Ok(StealthInputsStatement {
        inputs,
        revealed_amount: Amount::from_u64(intent.revealed_input_amount),
    })
}

/// Assembles the complete [`StealthTransferStatement`] from the outputs statement and the input
/// resolution result, generates + locally validates the balance proof, runs the `validate_transfer`
/// pre-flight, and builds the [`UnsignedTransaction`].
///
/// Pure: the only randomness is inside the reused balance-proof crypto leaf (see the module docs).
///
/// `agg_input_mask` and `sig_reqs_state` are produced by the input resolver; `outputs_statement` +
/// `agg_output_mask` by the outputs path. The `resolved_utxo_inputs` are the resolved UTXO substate
/// requirements that must be attached to the transaction as inputs.
#[allow(clippy::too_many_arguments)]
pub fn assemble_stealth_transfer_statement(
    network: Network,
    intent: &StealthTransferIntent,
    outputs_statement: StealthOutputsStatement,
    agg_output_mask: SecretKeyBytes,
    agg_input_mask: RistrettoSecretKey,
    sig_reqs_state: StealthSignatureRequirementsState,
    resolved_utxo_inputs: Vec<tari_ootle_common_types::InputDeclaration>,
    _entropy: &StealthEntropy,
) -> Result<StealthPartialTransaction, OotleSdkError> {
    // Pre-flight: the per-output revealed-deposit slices must reconcile with the top-level
    // `revealed_output_amount` the balance proof commits to. Reject a mismatched intent before
    // assembling rather than emitting a statement the engine would reject.
    intent.validate_revealed_outputs()?;

    let inputs_statement = build_inputs_statement(intent)?;
    let agg_output_mask = parse_mask(&agg_output_mask)?;

    // A balance proof is required iff there is at least one stealth input or output. A revealed-only
    // transfer needs no balance proof.
    let requires_balance_proof = !inputs_statement.inputs.is_empty() || !outputs_statement.outputs.is_empty();

    // This path spends only key-path inputs, so no partition gates on a covenant and there is nothing to claim. The
    // claims are bound by the balance proof, so they must be settled before it is signed.
    let covenant_claims = vec![];

    // Generate the balance proof (not byte-stable — internal RNG nonce).
    let balance_proof = requires_balance_proof.then(|| {
        generate_stealth_balance_proof_signature(
            &agg_input_mask,
            &agg_output_mask,
            &inputs_statement,
            &outputs_statement,
            &covenant_claims,
        )
    });

    // Local pre-flight: the inputs and outputs must balance. The only way a freshly-generated proof
    // fails to verify is an input/output value imbalance.
    if let Some(balance_proof) = &balance_proof &&
        !validate_balance_proof_signature(balance_proof, &inputs_statement, &outputs_statement, &covenant_claims)
    {
        return Err(OotleSdkError::Stealth("inputs and outputs do not balance".to_string()));
    }

    // Assemble the full transfer statement.
    let transfer = StealthTransferStatement {
        inputs_statement,
        outputs_statement,
        balance_proof,
        covenant_claims,
    };

    // Full pre-flight: the engine's own validation must accept the statement. `None` view key — the
    // per-output view-key proofs are validated on the outputs path; here we validate the
    // transfer-level balance.
    validate_transfer(&transfer, None)
        .map_err(|e| OotleSdkError::Stealth(format!("transfer validation failed: {e}")))?;

    // Build the unsigned transaction around the assembled statement.
    let unsigned = build_unsigned_transaction(network, intent, transfer, resolved_utxo_inputs)?;

    Ok(StealthPartialTransaction {
        unsigned,
        sig_reqs: sig_reqs_state,
    })
}

/// Emits the verified stealth-transfer instruction sequence onto a fresh builder and folds in the
/// resolved UTXO inputs + epoch/dry-run metadata.
///
/// Instruction sequence: fee →
/// (optional withdraw → put-on-workspace) → stealth-transfer → (optional revealed-output deposit) →
/// inputs → build. When `intent.revealed_input_amount > 0`, a prior `withdraw` produces the
/// revealed-input bucket the `StealthTransfer` instruction consumes; otherwise no bucket is wired.
///
/// ## Revealed-output bucket deposit
///
/// When `intent.revealed_output_amount > 0`, the `StealthTransfer` instruction returns the revealed
/// (plaintext) value as its last output; this is captured on the workspace as `"output_bucket"` and
/// then deposited per-output into each recipient account:
/// - a **single** output deposits the whole `"output_bucket"` via
///   [`create_account_with_bucket`](tari_ootle_transaction::TransactionBuilder::create_account_with_bucket), keyed by
///   that output's `destination_account_pk`;
/// - **multiple** outputs each `take_from_bucket("output_bucket", revealed_amount, "output-sub-bucket-{i}")` then
///   `create_account_with_bucket(dest_pk, "output-sub-bucket-{i}")`, keyed by output index `i`.
///
/// The per-output `revealed_amount` slices are validated to sum to `revealed_output_amount`
/// (`intent.validate_revealed_outputs`) before assembly. This is pure instruction assembly — no new
/// randomness — so both the deterministic and production paths share it.
fn build_unsigned_transaction(
    network: Network,
    intent: &StealthTransferIntent,
    transfer: StealthTransferStatement,
    resolved_utxo_inputs: Vec<tari_ootle_common_types::InputDeclaration>,
) -> Result<UnsignedTransaction, OotleSdkError> {
    let from_component: ComponentAddress = intent.from_account.to_internal()?;
    let resource: ResourceAddress = intent.resource_address.to_internal()?;
    let fee = intent.fee.to_internal();
    let revealed_amount = intent.revealed_input_amount;

    let mut builder = TransactionBuilder::new(network.as_byte(), tari_ootle_common_types::Epoch(intent.max_epoch));
    builder = builder.pay_fee_from_component(from_component, fee);

    builder = if revealed_amount > 0 {
        // The revealed input enters as a bucket from a prior withdraw. The bucket label must be
        // derived via `bucket_label(&builder)` at this exact point in the call chain so its numeric
        // workspace id matches what the encoded instruction carries.
        let bucket = bucket_label(&builder);
        builder
            .call_method(from_component, "withdraw", args![
                resource,
                Amount::from_u64(revealed_amount)
            ])
            .put_last_instruction_output_on_workspace(&bucket)
            .stealth_transfer_with_opt_input_bucket(resource, transfer, Some(bucket))
    } else {
        builder.stealth_transfer(resource, transfer)
    };

    // Revealed-output deposit: when the transfer pays revealed (plaintext) value, the
    // `StealthTransfer` instruction returns it as a bucket. Capture it on the workspace and deposit
    // it per-output into each recipient account. The per-output `revealed_amount` slices were
    // validated to sum to `revealed_output_amount` upstream.
    if intent.revealed_output_amount > 0 {
        builder = builder.put_last_instruction_output_on_workspace("output_bucket");
        let needs_to_split = intent.outputs.len() > 1;
        for (i, output) in intent.outputs.iter().enumerate() {
            if output.revealed_amount == 0 {
                continue;
            }
            let dest_pk = output.destination_account_pk.to_internal();
            builder = if needs_to_split {
                let sub_bucket = format!("output-sub-bucket-{i}");
                builder
                    .take_from_bucket(
                        "output_bucket",
                        Amount::from_u64(output.revealed_amount),
                        sub_bucket.clone(),
                    )
                    .create_account_with_bucket(dest_pk, sub_bucket)
            } else {
                builder.create_account_with_bucket(dest_pk, "output_bucket")
            };
        }
    }

    // Epoch window + dry-run flag.
    builder = builder
        .with_min_epoch(intent.min_epoch.map(tari_ootle_common_types::Epoch))
        .with_dry_run(intent.dry_run);

    // Attach the resolved input substates (the from-account component + its vault, resolved via the
    // account wants, plus any stealth-input UTXOs). Without these the engine rejects the tx for
    // missing input substates. The resource input is auto-added by `stealth_transfer*` — do not
    // re-add it.
    builder = builder.with_inputs(resolved_utxo_inputs);

    Ok(builder.build_unsigned())
}

/// Folds the resolver's accumulated stealth state into a [`StealthSignatureRequirementsState`].
///
/// When the account key must sign, every spend signer is a *required* signer (the seal is the account
/// key, so `seal_signer` stays `None`); otherwise the first input is the seal signer and the rest are
/// required.
fn signature_requirements_from_partial(partial: &PartialTransaction) -> StealthSignatureRequirementsState {
    StealthSignatureRequirementsState {
        must_sign_with_account_key: partial.must_sign_with_account_key(),
        seal_signer: partial.seal_signer().cloned(),
        other_signers: partial.required_signers().to_vec(),
    }
}

/// The stealth build context the resolver carries across the host-driven fetch loop so the **final**
/// [`apply_fetched_substates_stealth`] can assemble the transfer statement + balance proof without the
/// host re-passing the intent or pinned entropy. Stashed in the resolver
/// [`PartialTransaction`](crate::inputs::PartialTransaction) at seed time.
#[derive(Debug, Clone)]
pub struct StealthBuildCtx {
    /// The transfer intent (inputs/outputs/revealed amounts/epoch window) — assembly re-reads it.
    pub intent: StealthTransferIntent,
    /// The pinned proof + seal entropy. Outputs statement + balance proof need it at assembly time,
    /// which (in the looped path) happens during the final `apply`, not `build`.
    pub entropy: StealthEntropy,
}

/// The outcome of one stealth `apply` round.
///
/// A `NeedMore` carries the **concrete** next-fetch ids the resolver discovered (so a thin host
/// converges by fetching exactly them), or `Resolved` carries the assembled, ready-to-seal
/// [`StealthPartialTransaction`].
#[derive(Debug)]
pub enum StealthResolution {
    /// More substates are needed — fetch `fetch_ids` and call [`apply_fetched_substates_stealth`]
    /// again on the returned `partial`.
    NeedMore {
        /// The still-in-progress resolver, carrying the cache + accumulated stealth state + the
        /// stashed [`StealthBuildCtx`].
        partial: Box<PartialTransaction>,
        /// The concrete substate ids the host must fetch next (the authoritative next-fetch set).
        fetch_ids: Vec<String>,
    },
    /// All stealth inputs resolved — the assembled transfer is ready to seal.
    Resolved(Box<StealthPartialTransaction>),
}

/// Seed the host-driven resolver + the stealth-UTXO want list (seed-reproducible: the build seed
/// expands into the stashed proof + seal entropy).
///
/// The host then drives the fetch loop: fetch the want-list seed ids, call
/// [`apply_fetched_substates_stealth`], and on `NeedMore` fetch the returned `fetch_ids` and repeat
/// until `Resolved`. `intent` + the expanded entropy are stashed in the returned resolver (via
/// [`StealthBuildCtx`]) so the final `apply` can assemble without the host re-passing them — the host
/// never derives ids or re-supplies the build inputs.
pub fn build_stealth_unsigned_with_wants_with_seed(
    network: Network,
    intent: &StealthTransferIntent,
    seed: &BuildSeed,
) -> Result<(PartialTransaction, WantList), OotleSdkError> {
    seed.validate_nonzero()?;
    let entropy = StealthEntropy::from_seed(seed, intent.outputs.len());
    // The live driver resolves the from-account component + vault (the fee + optional revealed-input
    // withdraw reference the account) in addition to the stealth-UTXO inputs, so all are declared.
    let want_list = WantList::from_stealth_inputs_with_account(intent);
    let partial = seed_partial(network, intent, &entropy, want_list.clone())?;
    Ok((partial, want_list))
}

/// The **random-nonce default** seed: like [`build_stealth_unsigned_with_wants_with_seed`] but expands
/// the proof + seal entropy from a fresh OS-RNG seed and stashes it in the resolver, so the eventual
/// sealed bytes are **not** reproducible (the safe default for real submission). The host drives the
/// same fetch loop; the entropy never crosses the boundary.
pub fn build_stealth_unsigned_with_wants(
    network: Network,
    intent: &StealthTransferIntent,
) -> Result<(PartialTransaction, WantList), OotleSdkError> {
    let entropy = StealthEntropy::from_os_rng(intent.outputs.len());
    let want_list = WantList::from_stealth_inputs_with_account(intent);
    let partial = seed_partial(network, intent, &entropy, want_list.clone())?;
    Ok((partial, want_list))
}

/// Fold a fetched batch into the resolver, returning either a `NeedMore` with the
/// next-fetch ids (`NeedMore` is **never** remapped to an error) or, once every stealth input
/// resolves, the **assembled** [`StealthPartialTransaction`] ready to seal.
///
/// Internally uses [`apply_fetched_substates_with_secrets`] (so vault/UTXO discovery stays in the
/// core); on completion it pulls the stashed [`StealthBuildCtx`] out of the resolver and runs the
/// shared assembly tail [`assemble_resolved_stealth`]. **Consumes** `partial` on every path.
pub fn apply_fetched_substates_stealth(
    mut partial: PartialTransaction,
    network: Network,
    fetched: &[FetchedSubstate],
    spend_secrets: &[SecretKeyBytes],
    keys: &StealthKeys,
) -> Result<StealthResolution, OotleSdkError> {
    // The stashed context (intent + entropy) must be present for assembly on the final round; take it
    // out and put it back if this round is not yet resolved (so the next round still has it).
    let ctx = partial.take_stealth_ctx().ok_or_else(|| {
        OotleSdkError::Resolution(
            "stealth apply called on a partial that was not seeded by build_stealth_unsigned_with_wants".to_string(),
        )
    })?;
    let secrets = spend_secrets_map(&ctx.intent, spend_secrets)?;

    match apply_fetched_substates_with_secrets(partial, fetched, &secrets)? {
        Resolution::Resolved(resolved) => {
            let assembled = assemble_resolved_stealth(network, &ctx.intent, resolved, &ctx.entropy, keys)?;
            Ok(StealthResolution::Resolved(Box::new(assembled)))
        },
        Resolution::NeedMore {
            mut partial, fetch_ids, ..
        } => {
            // Not yet resolved — re-stash the context so the next round can assemble.
            partial.restore_stealth_ctx(ctx);
            Ok(StealthResolution::NeedMore {
                partial: Box::new(partial),
                fetch_ids,
            })
        },
    }
}

/// Builds a placeholder unsigned transaction + stashes the build context (`intent` + `entropy`) in a
/// fresh resolver seeded with `wants`, so the final `apply` can assemble without the host re-passing
/// the build inputs.
fn seed_partial(
    network: Network,
    intent: &StealthTransferIntent,
    entropy: &StealthEntropy,
    wants: WantList,
) -> Result<PartialTransaction, OotleSdkError> {
    // The unsigned the partial carries is a placeholder — the real instruction sequence is built in
    // `build_unsigned_transaction` once the inputs/outputs statements exist (see
    // `assemble_resolved_stealth`). We only need the partial's resolver machinery + accumulated stealth
    // state + the stashed build context, so an empty unsigned suffices.
    let unsigned =
        TransactionBuilder::new(network.as_byte(), tari_ootle_common_types::Epoch(intent.max_epoch)).build_unsigned();
    let ctx = StealthBuildCtx {
        intent: intent.clone(),
        entropy: entropy.clone(),
    };
    Ok(PartialTransaction::new_for_stealth_inputs_with_wants(
        unsigned, intent, ctx, wants,
    ))
}

/// Resolves the stealth inputs from a caller-supplied set of fetched substates in one shot, by driving
/// the **same** host-driven loop the looped path uses, only with every UTXO supplied up front.
///
/// The one-shot full-pipeline entry points (and the C ABI one-shot) fetch every input UTXO before the
/// call, so this drives the resolver to completion internally: first pass over the supplied batch, then
/// (if `NeedMore`) one empty-batch flush that marks any still-outstanding id as definitively absent —
/// surfacing a clear `Resolution` error for a missing required UTXO rather than looping.
fn resolve_stealth_inputs(
    network: Network,
    intent: &StealthTransferIntent,
    entropy: &StealthEntropy,
    fetched_utxos: &[FetchedSubstate],
    secrets: &HashMap<String, SecretKeyBytes>,
) -> Result<PartialTransaction, OotleSdkError> {
    // The one-shot path takes every input substate up front (it cannot drive a host fetch loop), so
    // it resolves only the bare stealth-UTXO wants; the from-account component + vault are declared by
    // the live driver's account wants ([`build_stealth_unsigned_with_wants`]).
    let wants = WantList::from_stealth_inputs(intent);
    let mut partial = seed_partial(network, intent, entropy, wants.clone())?;
    // The one-shot path assembles directly from the resolver below; the stashed ctx is unused here.
    drop(partial.take_stealth_ctx());

    // Short-circuit: no stealth inputs ⇒ nothing to resolve.
    if wants.0.is_empty() {
        return Ok(partial);
    }

    // The caller supplies every stealth-input UTXO substate up front, so the first pass resolves them
    // all. A `NeedMore` means a required UTXO was not provided. Re-applying an empty batch flushes the
    // outstanding fetch ids to `Some(None)` (definitively absent), which makes the resolver surface its
    // own `OotleSdkError::Invalid("stealth UTXO … not found")`. We remap any such failure to a single,
    // unambiguous `Resolution` error so a host can tell "you didn't supply every input" apart from a
    // genuine on-chain absence on the first pass.
    match apply_fetched_substates_with_secrets(partial, fetched_utxos, secrets)? {
        Resolution::Resolved(p) => Ok(p),
        Resolution::NeedMore { partial: p, .. } => {
            let missing_inputs =
                || OotleSdkError::Resolution("not all stealth UTXO inputs were provided up front".to_string());
            match apply_fetched_substates_with_secrets(p, &[], secrets).map_err(|_| missing_inputs())? {
                Resolution::Resolved(p) => Ok(p),
                Resolution::NeedMore { .. } => Err(missing_inputs()),
            }
        },
    }
}

/// The key authorised to take this transfer's revealed output, or `None` when it reveals nothing.
///
/// The ephemeral seal case draws its key at seal time and has neither stealth inputs nor a revealed input, so a
/// statement built under it carries no spendable input and cannot reveal an output either — a revealed output there
/// is an error, not a key to invent.
fn resolve_revealed_receiver(
    network: Network,
    intent: &StealthTransferIntent,
    sig_reqs: &StealthSignatureRequirementsState,
    keys: &StealthKeys,
) -> Result<Option<RistrettoPublicKeyBytes>, OotleSdkError> {
    if intent.revealed_output_amount == 0 {
        return Ok(None);
    }
    let secrets = SpendSecrets::parse(keys)?;
    let receiver = seal_public_key(network, &secrets, sig_reqs)?.ok_or_else(|| {
        OotleSdkError::Validation(
            "a revealed output needs a receiver that signs, but this transfer has no signer to seal with".to_string(),
        )
    })?;
    Ok(Some(receiver))
}

/// The shared assembly tail — runs the post-resolution work (outputs statement from `entropy`, balance
/// proof, the `validate_transfer` pre-flight, and building the [`StealthPartialTransaction`]) on a
/// fully-resolved resolver. Called by **both** the one-shot path
/// ([`build_stealth_transfer_unsigned_with_seed`]) **and** the looped path
/// ([`apply_fetched_substates_stealth`]), so there is exactly one assembly implementation.
fn assemble_resolved_stealth(
    network: Network,
    intent: &StealthTransferIntent,
    partial: PartialTransaction,
    entropy: &StealthEntropy,
    keys: &StealthKeys,
) -> Result<StealthPartialTransaction, OotleSdkError> {
    let sig_reqs = signature_requirements_from_partial(&partial);

    // The revealed output names the key that will take it, and the balance proof binds that name, so the receiver has
    // to be settled here rather than at seal time. Taking it from `seal_public_key` — the same derivation `plan_seal`
    // seals with — is what keeps the named receiver and the transaction's actual signer the same key.
    let revealed_receiver = resolve_revealed_receiver(network, intent, &sig_reqs, keys)?;

    // Outputs statement + aggregate output mask (expanded entropy).
    let (outputs_statement, agg_output_mask) =
        build_stealth_outputs_statement_from_entropy(network, intent, entropy, revealed_receiver)?;

    let agg_input_mask = partial.agg_input_mask().clone();
    let resolved_utxo_inputs = partial.resolved_inputs().to_vec();

    // Assemble + validate + build.
    assemble_stealth_transfer_statement(
        network,
        intent,
        outputs_statement,
        agg_output_mask,
        agg_input_mask,
        sig_reqs,
        resolved_utxo_inputs,
        entropy,
    )
}

/// The deterministic full-pipeline entry point: outputs, inputs, and assembly in one call.
///
/// Builds the outputs statement (injected entropy), resolves + decrypts the stealth inputs
/// (caller-supplied spend secrets carried in `intent.inputs` + `spend_secrets`), then assembles +
/// validates the transfer statement and the unsigned transaction. The `fetched_utxos` are every
/// stealth-input UTXO substate, fetched up front by the caller.
///
/// **`spend_secrets`** are the per-input view-only secrets (one per `intent.inputs`, positional),
/// used to decrypt the input masks. They are borrowed for the call only and never stored.
///
/// The seed-reproducible path calls no RNG of its own; the bulletproof + balance-proof nonces come
/// from the reused crypto leaves, so the proofs are not byte-stable (semantic comparison only).
pub fn build_stealth_transfer_unsigned_with_seed(
    network: Network,
    intent: &StealthTransferIntent,
    fetched_utxos: &[FetchedSubstate],
    spend_secrets: &[SecretKeyBytes],
    keys: &StealthKeys,
    seed: &BuildSeed,
) -> Result<StealthPartialTransaction, OotleSdkError> {
    seed.validate_nonzero()?;
    let entropy = StealthEntropy::from_seed(seed, intent.outputs.len());

    // Resolve + decrypt the stealth inputs (drives the resolver to completion over the up-front
    // batch).
    let secrets = spend_secrets_map(intent, spend_secrets)?;
    let partial = resolve_stealth_inputs(network, intent, &entropy, fetched_utxos, &secrets)?;

    // The shared assembly tail (also used by the looped `apply` path).
    assemble_resolved_stealth(network, intent, partial, &entropy, keys)
}

/// The random-nonce default full-pipeline entry point: expands the proof entropy from a fresh OS-RNG
/// seed. The resulting statement is **not** reproducible (the safe default for real submission).
pub fn build_stealth_transfer_unsigned(
    network: Network,
    intent: &StealthTransferIntent,
    fetched_utxos: &[FetchedSubstate],
    spend_secrets: &[SecretKeyBytes],
    keys: &StealthKeys,
) -> Result<StealthPartialTransaction, OotleSdkError> {
    let entropy = StealthEntropy::from_os_rng(intent.outputs.len());
    let secrets = spend_secrets_map(intent, spend_secrets)?;
    let partial = resolve_stealth_inputs(network, intent, &entropy, fetched_utxos, &secrets)?;
    assemble_resolved_stealth(network, intent, partial, &entropy, keys)
}

#[cfg(test)]
mod tests {
    use ootle_byte_type::ToByteType;
    use tari_crypto::{
        keys::PublicKey as _,
        ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    };
    use tari_ootle_transaction::Instruction;

    use super::*;
    use crate::{
        inputs::StealthSignerEntry,
        types::{
            address::{ComponentAddressStr, ResourceAddressStr},
            bytes::PublicKeyBytes,
            numeric::BoundaryAmount,
            stealth::{StealthInputSpec, StealthOutputSpec, StealthPayTo},
        },
    };

    fn pk_bytes(seed: u8) -> PublicKeyBytes {
        let mut b = [0u8; 32];
        b[0] = seed;
        let sk = RistrettoSecretKey::from_canonical_bytes(&b).unwrap();
        let pk = RistrettoPublicKey::from_secret_key(&sk);
        PublicKeyBytes::from_bytes(pk.as_bytes()).unwrap()
    }

    fn secret(seed: u8) -> SecretKeyBytes {
        let mut b = [0u8; 32];
        b[0] = seed;
        RistrettoSecretKey::from_canonical_bytes(&b).expect("canonical low scalar");
        SecretKeyBytes::from_array(b)
    }

    fn tari_resource() -> ResourceAddressStr {
        ResourceAddressStr::parse(tari_template_lib_types::constants::STEALTH_TARI_RESOURCE_ADDRESS.to_string())
            .unwrap()
    }

    fn from_component() -> ComponentAddressStr {
        ComponentAddressStr::parse(
            tari_template_lib_types::ComponentAddress::new(tari_template_lib_types::ObjectKey::from_array(
                [0xaa; tari_template_lib_types::ObjectKey::LENGTH],
            ))
            .to_string(),
        )
        .unwrap()
    }

    fn output_spec(amount: u64) -> StealthOutputSpec {
        StealthOutputSpec {
            destination_account_pk: pk_bytes(3),
            destination_view_pk: pk_bytes(4),
            amount,
            revealed_amount: 0,
            resource_address: tari_resource(),
            resource_view_key: None,
            memo: None,
            pay_to: StealthPayTo::StealthPublicKey,
            utxo_tag: None,
            minimum_value_promise: 0,
        }
    }

    /// A fixed build seed for the seed-reproducible paths; `byte` distinguishes per-test seeds.
    fn seed_for(byte: u8) -> BuildSeed {
        BuildSeed::from_array([byte; 32])
    }

    /// Expands a fixed build seed into a `num_outputs`-wide entropy bundle (mirrors what the public
    /// seed paths derive internally), for the internal entropy-taking assembly helpers.
    fn entropy_for(num_outputs: usize, byte: u8) -> StealthEntropy {
        StealthEntropy::from_seed(&seed_for(byte), num_outputs)
    }

    /// A fixed vault id for the from-account in the looped-resolution tests.
    fn from_vault_id() -> tari_template_lib_types::VaultId {
        tari_template_lib_types::VaultId::new(tari_template_lib_types::ObjectKey::from_array(
            [0xcc; tari_template_lib_types::ObjectKey::LENGTH],
        ))
    }

    /// The from-account component substate (JSON, as the indexer returns it), referencing `from_vault_id`.
    /// The live driver's account wants resolve this to declare the component + vault as inputs.
    fn from_account_component_json() -> serde_json::Value {
        use tari_engine_types::{
            component::{Component, ComponentBody, ComponentHeader},
            substate::SubstateValue,
        };
        use tari_template_lib_types::{EntityId, SubstateOwnerRule, access_rules::ComponentAccessRules};

        let vault_ids = [from_vault_id()];
        let state = tari_bor::to_value(vault_ids.as_slice()).unwrap();
        let component = Component {
            header: ComponentHeader {
                template_address: tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS,
                owner_rule: SubstateOwnerRule::None,
                access_rules: ComponentAccessRules::new(),
                entity_id: EntityId::from_array([0u8; EntityId::LENGTH]),
            },
            body: ComponentBody::from_cbor_value(state),
        };
        crate::substate_decode::indexer_substate_json(&SubstateValue::Component(component))
    }

    /// The from-account's vault substate (JSON), holding the tari resource the transfer pays in.
    fn from_account_vault_json() -> serde_json::Value {
        use tari_engine_types::{resource_container::ResourceContainer, substate::SubstateValue, vault::Vault};
        use tari_template_lib_types::Amount;

        let resource = tari_resource().as_str().parse().unwrap();
        let vault = Vault::new(ResourceContainer::public_fungible(resource, Amount::new(1_000_000)));
        crate::substate_decode::indexer_substate_json(&SubstateValue::Vault(vault))
    }

    /// Serves a fetch request for `id` from the from-account substate store (component + vault), or
    /// `None` for any other id (e.g. a stealth UTXO the caller supplies separately).
    fn account_substate_for(id: &str) -> Option<FetchedSubstate> {
        let component_id = from_component().as_str().to_string();
        let vault_id = tari_engine_types::substate::SubstateId::Vault(from_vault_id()).to_string();
        if id == component_id {
            Some(FetchedSubstate {
                substate_id: component_id,
                version: 0,
                substate_value: from_account_component_json(),
            })
        } else if id == vault_id {
            Some(FetchedSubstate {
                substate_id: vault_id,
                version: 0,
                substate_value: from_account_vault_json(),
            })
        } else {
            None
        }
    }

    /// An intent: one stealth output (value `out_amount`) balanced by a revealed input bucket. No
    /// stealth inputs (the simplest balanced statement with a real balance proof + a withdraw bucket).
    fn balanced_intent(out_amount: u64, revealed_input: u64) -> StealthTransferIntent {
        StealthTransferIntent {
            from_account: from_component(),
            resource_address: tari_resource(),
            fee: BoundaryAmount::new(2000),
            inputs: Vec::<StealthInputSpec>::new(),
            outputs: vec![output_spec(out_amount)],
            revealed_input_amount: revealed_input,
            revealed_output_amount: 0,
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
            pay_fee_from_revealed: false,
        }
    }

    /// Drives the outputs + assembly paths directly with an explicit entropy bundle (no stealth inputs
    /// ⇒ no fetch loop). Mirrors `build_stealth_transfer_unsigned_with_seed`'s body but feeds a chosen
    /// entropy, so a test can compare against a direct outputs-statement build using the same bundle.
    fn assemble_from_intent(
        intent: &StealthTransferIntent,
        entropy: &StealthEntropy,
    ) -> Result<StealthPartialTransaction, OotleSdkError> {
        let secrets = spend_secrets_map(intent, &[])?;
        let partial = resolve_stealth_inputs(Network::LocalNet, intent, entropy, &[], &secrets)?;
        assemble_resolved_stealth(Network::LocalNet, intent, partial, entropy, &test_keys())
    }

    /// The account key these tests seal with. A revealed output names its public key, since that is the key
    /// `plan_seal` will seal with for an intent that draws on the account.
    fn test_keys() -> StealthKeys {
        StealthKeys::new(
            SecretKeyBytes::from_bytes(RistrettoSecretKey::from(42u64).as_bytes()).unwrap(),
            SecretKeyBytes::from_bytes(RistrettoSecretKey::from(43u64).as_bytes()).unwrap(),
        )
    }

    /// The public key [`test_keys`] seals with — the receiver these tests expect on a revealed output.
    fn test_account_pk() -> RistrettoPublicKeyBytes {
        RistrettoPublicKeyBytes::from_bytes(
            tari_crypto::ristretto::RistrettoPublicKey::from_secret_key(&RistrettoSecretKey::from(42u64)).as_bytes(),
        )
        .unwrap()
    }

    fn stealth_transfer_instructions(unsigned: &UnsignedTransaction) -> Vec<&Instruction> {
        unsigned
            .instructions()
            .iter()
            .filter(|i| matches!(i, Instruction::StealthTransfer { .. }))
            .collect()
    }

    // (a) Balanced statement → Ok with exactly one StealthTransfer instruction.
    #[test]
    fn balanced_statement_assembles_one_stealth_transfer() {
        let intent = balanced_intent(1_000_000, 1_000_000);
        let entropy = entropy_for(1, 10);
        let partial = assemble_from_intent(&intent, &entropy).unwrap();
        let unsigned = partial.unsigned();
        assert_eq!(
            stealth_transfer_instructions(unsigned).len(),
            1,
            "exactly one StealthTransfer instruction"
        );
    }

    // (b) Unbalanced → OotleSdkError::Stealth (code STEALTH).
    #[test]
    fn unbalanced_statement_is_stealth_error() {
        // Output value 1_000_000 but only 999_999 revealed in ⇒ does not balance.
        let intent = balanced_intent(1_000_000, 999_999);
        let entropy = entropy_for(1, 20);
        let err = assemble_from_intent(&intent, &entropy).unwrap_err();
        assert_eq!(err.code(), "STEALTH");
    }

    // (c) Revealed-only → balance proof is None.
    #[test]
    fn revealed_only_has_no_balance_proof() {
        // No stealth inputs, no stealth outputs: revealed in == revealed out.
        let intent = StealthTransferIntent {
            from_account: from_component(),
            resource_address: tari_resource(),
            fee: BoundaryAmount::new(2000),
            inputs: Vec::<StealthInputSpec>::new(),
            outputs: Vec::<StealthOutputSpec>::new(),
            revealed_input_amount: 500_000,
            revealed_output_amount: 500_000,
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
            pay_fee_from_revealed: false,
        };
        let entropy = entropy_for(0, 30);
        let partial = assemble_from_intent(&intent, &entropy).unwrap();
        // Decode the StealthTransfer instruction and confirm the balance proof is None.
        let unsigned = partial.unsigned();
        let txs = stealth_transfer_instructions(unsigned);
        assert_eq!(txs.len(), 1);
        if let Instruction::StealthTransfer { statement, .. } = txs[0] {
            assert!(statement.balance_proof.is_none(), "revealed-only ⇒ no balance proof");
            assert!(statement.inputs_statement.inputs.is_empty());
            assert!(statement.outputs_statement.outputs.is_empty());
        } else {
            panic!("expected StealthTransfer");
        }
    }

    // (d) Revealed-input path → withdraw + put-on-workspace + StealthTransfer{ Some bucket }.
    #[test]
    fn revealed_input_wires_withdraw_bucket() {
        let intent = balanced_intent(1_000_000, 1_000_000);
        let entropy = entropy_for(1, 40);
        let partial = assemble_from_intent(&intent, &entropy).unwrap();
        let unsigned = partial.unsigned();
        let instrs = unsigned.instructions();

        assert!(
            instrs
                .iter()
                .any(|i| matches!(i, Instruction::CallMethod { method, .. } if method.as_ref() == "withdraw")),
            "a withdraw CallMethod must precede the stealth transfer"
        );
        assert!(
            instrs
                .iter()
                .any(|i| matches!(i, Instruction::PutLastInstructionOutputOnWorkspace { .. })),
            "the withdraw output must be put on the workspace"
        );
        let txs = stealth_transfer_instructions(unsigned);
        assert_eq!(txs.len(), 1);
        if let Instruction::StealthTransfer {
            revealed_input_bucket, ..
        } = txs[0]
        {
            assert!(revealed_input_bucket.is_some(), "revealed input ⇒ a bucket is wired");
        } else {
            panic!("expected StealthTransfer");
        }
    }

    // (e) Stealth-only (no revealed input) → no bucket. Balance one stealth input against one stealth
    //     output of equal value (the only no-revealed-input balanced shape), assembling the statement
    //     directly with a fabricated balanced input so the builder takes its `else` (no-bucket) arm.
    #[test]
    fn stealth_only_has_no_input_bucket() {
        use tari_engine_types::crypto::commit_u64_amount;

        let value = 1_000_000u64;
        let entropy = entropy_for(1, 50);

        // Output side: gives the output commitment + the aggregate output mask.
        let intent_out = balanced_intent(value, 0); // revealed_input 0 ⇒ no bucket
        let (outputs_statement, agg_output_mask) = build_stealth_outputs_statement_from_entropy(
            Network::LocalNet,
            &intent_out,
            &entropy,
            Some(test_account_pk()),
        )
        .unwrap();

        // Input side: a fabricated stealth input of the same value with a freely-chosen mask.
        let input_mask = RistrettoSecretKey::from_canonical_bytes(secret(150).as_bytes()).unwrap();
        let input_commitment = commit_u64_amount(&input_mask, value);
        let commitment_bytes =
            crate::types::stealth::CommitmentBytes::from_hex(&hex::encode(input_commitment.to_byte_type().as_bytes()))
                .unwrap();

        let intent = StealthTransferIntent {
            from_account: from_component(),
            resource_address: tari_resource(),
            fee: BoundaryAmount::new(2000),
            inputs: vec![StealthInputSpec {
                commitment: commitment_bytes,
                owner_account_pk: pk_bytes(9),
            }],
            outputs: intent_out.outputs,
            revealed_input_amount: 0,
            revealed_output_amount: 0,
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
            pay_fee_from_revealed: false,
        };

        // The input is the (implicit) seal signer (not account-key-signed, first input).
        let sig_reqs = StealthSignatureRequirementsState {
            must_sign_with_account_key: false,
            seal_signer: Some(StealthSignerEntry {
                account_pk_hex: pk_bytes(9).to_hex(),
                public_nonce_bytes: pk_bytes(8),
            }),
            other_signers: Vec::new(),
        };

        let partial = assemble_stealth_transfer_statement(
            Network::LocalNet,
            &intent,
            outputs_statement,
            agg_output_mask,
            input_mask,
            sig_reqs,
            Vec::new(),
            &entropy,
        )
        .unwrap();

        let unsigned = partial.unsigned();
        // No withdraw, no put-on-workspace (no revealed input bucket).
        assert!(
            !unsigned
                .instructions()
                .iter()
                .any(|i| matches!(i, Instruction::PutLastInstructionOutputOnWorkspace { .. }))
        );
        let txs = stealth_transfer_instructions(unsigned);
        assert_eq!(txs.len(), 1);
        if let Instruction::StealthTransfer {
            revealed_input_bucket,
            statement,
            ..
        } = txs[0]
        {
            assert!(revealed_input_bucket.is_none(), "no revealed input ⇒ no bucket");
            assert_eq!(statement.inputs_statement.inputs.len(), 1);
            assert!(statement.balance_proof.is_some());
        } else {
            panic!("expected StealthTransfer");
        }

        // The signing requirements survive to the sign/seal path: the input is the seal signer.
        assert!(!partial.signature_requirements().must_sign_with_account_key);
        assert!(partial.signature_requirements().seal_signer.is_some());
    }

    // (f) Parity: the assembled statement's deterministic fields match a direct outputs-statement +
    //     inputs-statement build (the assembled StealthTransferStatement embeds exactly them).
    #[test]
    fn statement_fields_match_direct_build() {
        let intent = balanced_intent(1_000_000, 1_000_000);
        let entropy = entropy_for(1, 60);

        // Direct outputs statement.
        let (direct_outputs, _) =
            build_stealth_outputs_statement_from_entropy(Network::LocalNet, &intent, &entropy, Some(test_account_pk()))
                .unwrap();
        let direct_inputs = build_inputs_statement(&intent).unwrap();

        let partial = assemble_from_intent(&intent, &entropy).unwrap();
        let txs = stealth_transfer_instructions(partial.unsigned());
        if let Instruction::StealthTransfer { statement, .. } = txs[0] {
            // The output commitments + count match (agg_range_proof is non-byte-stable).
            assert_eq!(
                statement.outputs_statement.outputs, direct_outputs.outputs,
                "embedded outputs must match the direct build"
            );
            assert_eq!(statement.inputs_statement, direct_inputs);
            assert!(statement.balance_proof.is_some());
        } else {
            panic!("expected StealthTransfer");
        }
    }

    // (g) Reproducibility: two deterministic runs produce identical unsigned bytes modulo the
    //     non-byte-stable proofs. We compare the deterministic surface: the instruction sequence
    //     with the agg_range_proof + balance_proof nulled.
    #[test]
    fn deterministic_runs_match_modulo_unstable_proofs() {
        let intent = balanced_intent(1_000_000, 1_000_000);
        let entropy = entropy_for(1, 70);
        let a = assemble_from_intent(&intent, &entropy).unwrap();
        let b = assemble_from_intent(&intent, &entropy).unwrap();

        let txa = canonicalize(a.unsigned());
        let txb = canonicalize(b.unsigned());
        assert_eq!(txa, txb, "deterministic surface must be reproducible");

        // The structural (non-proof) statement fields must match exactly.
        let sa = transfer_statement(a.unsigned());
        let sb = transfer_statement(b.unsigned());
        assert_eq!(sa.inputs_statement, sb.inputs_statement);
        assert_eq!(sa.outputs_statement.outputs, sb.outputs_statement.outputs);
        assert_eq!(
            sa.outputs_statement.revealed_output,
            sb.outputs_statement.revealed_output
        );
    }

    fn transfer_statement(unsigned: &UnsignedTransaction) -> StealthTransferStatement {
        match stealth_transfer_instructions(unsigned)[0] {
            Instruction::StealthTransfer { statement, .. } => statement.clone(),
            _ => unreachable!(),
        }
    }

    /// Encodes the unsigned tx's instruction sequence with the two non-byte-stable proof fields zeroed
    /// (the `agg_range_proof` and the balance-proof signature), so two deterministic runs encode
    /// identically (semantic comparison). Mutates a cloned instruction `Vec` (the
    /// `UnsignedTransaction` exposes no in-place instruction accessor).
    fn canonicalize(unsigned: &UnsignedTransaction) -> Vec<u8> {
        let mut instrs: Vec<Instruction> = unsigned.instructions().to_vec();
        for instr in &mut instrs {
            if let Instruction::StealthTransfer { statement, .. } = instr {
                statement.outputs_statement.agg_range_proof = tari_template_lib_types::crypto::RangeProofBytes::empty();
                statement.balance_proof = None;
            }
        }
        // Fee instructions + epoch window/dry-run carry no proofs, so the cloned instruction sequence
        // (with proofs nulled) is the full deterministic surface for this transfer shape.
        let mut bytes = tari_bor::encode(&instrs).unwrap();
        bytes.extend_from_slice(&tari_bor::encode(&unsigned.fee_instructions().to_vec()).unwrap());
        bytes
    }

    // Full pipeline with a real fabricated stealth-input UTXO: the input is fetched,
    // decrypted, balanced against an equal-value stealth output, and the assembled statement validates.
    // Exercises `resolve_stealth_inputs`'s non-short-circuit path + the input-mask flow into the proof.
    #[test]
    fn full_pipeline_with_fabricated_utxo_input() {
        use tari_engine_types::{
            Utxo,
            UtxoOutput,
            crypto::{OutputBody, commit_u64_amount},
            substate::SubstateValue,
        };
        use tari_ootle_wallet_crypto::{encrypted_data::encrypt_data, kdfs, stealth::condition_root};
        use tari_template_lib_types::{
            access_rules::AccessRule,
            crypto::UtxoTag,
            stealth::{SpendAuthorization, SpendCondition},
        };

        use crate::{inputs::FetchedSubstate, stealth::inputs::stealth_utxo_substate_id};

        let value = 1_000_000u64;
        let seed = seed_for(80);

        // The stealth input: a known (value, mask) encrypted to a known view secret via a known nonce.
        let input_mask = RistrettoSecretKey::from_canonical_bytes(secret(160).as_bytes()).unwrap();
        let nonce_secret = RistrettoSecretKey::from_canonical_bytes(secret(161).as_bytes()).unwrap();
        let public_nonce = RistrettoPublicKey::from_secret_key(&nonce_secret);
        let view_secret = RistrettoSecretKey::from_canonical_bytes(secret(162).as_bytes()).unwrap();

        let commitment = commit_u64_amount(&input_mask, value).to_byte_type();
        let commitment_hex = hex::encode(commitment.as_bytes());

        let encryption_key = kdfs::encrypted_data_dh_kdf_aead(&view_secret, &public_nonce);
        let encrypted_data = encrypt_data(value, &input_mask, &encryption_key, None).unwrap();
        let utxo = Utxo::new(UtxoOutput {
            output: OutputBody {
                public_nonce: public_nonce.to_byte_type(),
                encrypted_data,
                minimum_value_promise: 0,
                viewable_balance: None,
            },
            auth: SpendAuthorization::Script(
                condition_root(&[SpendCondition::access_rule(AccessRule::AllowAll)]).unwrap(),
            ),
            tag: UtxoTag::new(0),
        });

        let owner_pk = pk_bytes(9);
        let substate_id = stealth_utxo_substate_id(tari_resource().as_str(), &commitment_hex).unwrap();
        let fetched = vec![FetchedSubstate {
            substate_id: substate_id.to_string(),
            version: 0,
            substate_value: crate::substate_decode::indexer_substate_json(&SubstateValue::Utxo(utxo)),
        }];

        let intent = StealthTransferIntent {
            from_account: from_component(),
            resource_address: tari_resource(),
            fee: BoundaryAmount::new(2000),
            inputs: vec![StealthInputSpec {
                commitment: crate::types::stealth::CommitmentBytes::from_hex(&commitment_hex).unwrap(),
                owner_account_pk: owner_pk,
            }],
            outputs: vec![output_spec(value)],
            revealed_input_amount: 0,
            revealed_output_amount: 0,
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
            pay_fee_from_revealed: false,
        };

        let spend_secrets = vec![SecretKeyBytes::from_bytes(view_secret.as_bytes()).unwrap()];
        let partial = build_stealth_transfer_unsigned_with_seed(
            Network::LocalNet,
            &intent,
            &fetched,
            &spend_secrets,
            &test_keys(),
            &seed,
        )
        .unwrap();

        // One StealthTransfer, no revealed-input bucket, balance proof present.
        let txs = stealth_transfer_instructions(partial.unsigned());
        assert_eq!(txs.len(), 1);
        if let Instruction::StealthTransfer {
            statement,
            revealed_input_bucket,
            ..
        } = txs[0]
        {
            assert!(revealed_input_bucket.is_none());
            assert_eq!(statement.inputs_statement.inputs.len(), 1);
            assert_eq!(statement.outputs_statement.outputs.len(), 1);
            assert!(statement.balance_proof.is_some());
        } else {
            panic!("expected StealthTransfer");
        }

        // The decrypted input became the (implicit) seal signer (no revealed input, first input).
        assert!(!partial.signature_requirements().must_sign_with_account_key);
        let seal = partial.signature_requirements().seal_signer.as_ref().unwrap();
        assert_eq!(seal.account_pk_hex, owner_pk.to_hex());

        // The resolved UTXO substate is attached as a transaction input.
        assert!(
            partial
                .unsigned()
                .inputs()
                .iter()
                .any(|i| i.substate_id().to_string() == substate_id.to_string())
        );
    }

    // `pay_fee_from_revealed` forces the account-key seal even with no revealed input: the same
    // single-stealth-input spend that otherwise promotes its input to seal signer (see
    // `full_pipeline_with_fabricated_utxo_input`) now seals with the account key and demotes the input
    // to a required signer, so the account can authorize `pay_fee`.
    #[test]
    fn pay_fee_from_revealed_forces_account_key_seal() {
        use tari_engine_types::{
            Utxo,
            UtxoOutput,
            crypto::{OutputBody, commit_u64_amount},
            substate::SubstateValue,
        };
        use tari_ootle_wallet_crypto::{encrypted_data::encrypt_data, kdfs, stealth::condition_root};
        use tari_template_lib_types::{
            access_rules::AccessRule,
            crypto::UtxoTag,
            stealth::{SpendAuthorization, SpendCondition},
        };

        use crate::{inputs::FetchedSubstate, stealth::inputs::stealth_utxo_substate_id};

        let value = 1_000_000u64;
        let seed = seed_for(80);

        let input_mask = RistrettoSecretKey::from_canonical_bytes(secret(160).as_bytes()).unwrap();
        let nonce_secret = RistrettoSecretKey::from_canonical_bytes(secret(161).as_bytes()).unwrap();
        let public_nonce = RistrettoPublicKey::from_secret_key(&nonce_secret);
        let view_secret = RistrettoSecretKey::from_canonical_bytes(secret(162).as_bytes()).unwrap();

        let commitment = commit_u64_amount(&input_mask, value).to_byte_type();
        let commitment_hex = hex::encode(commitment.as_bytes());

        let encryption_key = kdfs::encrypted_data_dh_kdf_aead(&view_secret, &public_nonce);
        let encrypted_data = encrypt_data(value, &input_mask, &encryption_key, None).unwrap();
        let utxo = Utxo::new(UtxoOutput {
            output: OutputBody {
                public_nonce: public_nonce.to_byte_type(),
                encrypted_data,
                minimum_value_promise: 0,
                viewable_balance: None,
            },
            auth: SpendAuthorization::Script(
                condition_root(&[SpendCondition::access_rule(AccessRule::AllowAll)]).unwrap(),
            ),
            tag: UtxoTag::new(0),
        });

        let owner_pk = pk_bytes(9);
        let substate_id = stealth_utxo_substate_id(tari_resource().as_str(), &commitment_hex).unwrap();
        let fetched = vec![FetchedSubstate {
            substate_id: substate_id.to_string(),
            version: 0,
            substate_value: crate::substate_decode::indexer_substate_json(&SubstateValue::Utxo(utxo)),
        }];

        // Even-balance confidential spend (input == output), no revealed bucket, pay_fee_from_revealed on.
        let intent = StealthTransferIntent {
            from_account: from_component(),
            resource_address: tari_resource(),
            fee: BoundaryAmount::new(2000),
            inputs: vec![StealthInputSpec {
                commitment: crate::types::stealth::CommitmentBytes::from_hex(&commitment_hex).unwrap(),
                owner_account_pk: owner_pk,
            }],
            outputs: vec![output_spec(value)],
            revealed_input_amount: 0,
            revealed_output_amount: 0,
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
            pay_fee_from_revealed: true,
        };

        let spend_secrets = vec![SecretKeyBytes::from_bytes(view_secret.as_bytes()).unwrap()];
        let partial = build_stealth_transfer_unsigned_with_seed(
            Network::LocalNet,
            &intent,
            &fetched,
            &spend_secrets,
            &test_keys(),
            &seed,
        )
        .unwrap();

        // The account key seals; the stealth input is demoted to a required signer (not the seal signer).
        let sig_reqs = partial.signature_requirements();
        assert!(sig_reqs.must_sign_with_account_key);
        assert!(sig_reqs.seal_signer.is_none());
        assert_eq!(sig_reqs.other_signers.len(), 1);
        assert_eq!(sig_reqs.other_signers[0].account_pk_hex, owner_pk.to_hex());
    }

    /// The host-driven two-phase loop (`build_stealth_unsigned_with_wants` →
    /// `apply_fetched_substates_stealth`×N) drives the from-account component + vault and a stealth
    /// input over **multiple fetch rounds**: the first empty batch returns `NeedMore { fetch_ids }`
    /// carrying the discovered UTXO id (NOT remapped to an error); later rounds supply the requested
    /// account substates + the UTXO until it resolves into an assembled `StealthPartialTransaction`.
    /// The assembled product matches the one-shot path's output for the same inputs (same
    /// deterministic statement fields).
    #[test]
    #[allow(clippy::too_many_lines)] // end-to-end fetch-loop test; clearer as one linear body
    fn two_phase_loop_converges_and_matches_one_shot() {
        use tari_engine_types::{
            Utxo,
            UtxoOutput,
            crypto::{OutputBody, commit_u64_amount},
            substate::SubstateValue,
        };
        use tari_ootle_wallet_crypto::{encrypted_data::encrypt_data, kdfs, stealth::condition_root};
        use tari_template_lib_types::{
            access_rules::AccessRule,
            crypto::UtxoTag,
            stealth::{SpendAuthorization, SpendCondition},
        };

        use crate::stealth::{assemble::StealthResolution, inputs::stealth_utxo_substate_id};

        let value = 1_000_000u64;
        let seed = seed_for(90);

        // The stealth input: a known (value, mask) encrypted to a known view secret via a known nonce.
        let input_mask = RistrettoSecretKey::from_canonical_bytes(secret(170).as_bytes()).unwrap();
        let nonce_secret = RistrettoSecretKey::from_canonical_bytes(secret(171).as_bytes()).unwrap();
        let public_nonce = RistrettoPublicKey::from_secret_key(&nonce_secret);
        let view_secret = RistrettoSecretKey::from_canonical_bytes(secret(172).as_bytes()).unwrap();

        let commitment = commit_u64_amount(&input_mask, value).to_byte_type();
        let commitment_hex = hex::encode(commitment.as_bytes());

        let encryption_key = kdfs::encrypted_data_dh_kdf_aead(&view_secret, &public_nonce);
        let encrypted_data = encrypt_data(value, &input_mask, &encryption_key, None).unwrap();
        let utxo = Utxo::new(UtxoOutput {
            output: OutputBody {
                public_nonce: public_nonce.to_byte_type(),
                encrypted_data,
                minimum_value_promise: 0,
                viewable_balance: None,
            },
            auth: SpendAuthorization::Script(
                condition_root(&[SpendCondition::access_rule(AccessRule::AllowAll)]).unwrap(),
            ),
            tag: UtxoTag::new(0),
        });

        let owner_pk = pk_bytes(9);
        let substate_id = stealth_utxo_substate_id(tari_resource().as_str(), &commitment_hex).unwrap();
        let fetched = vec![FetchedSubstate {
            substate_id: substate_id.to_string(),
            version: 0,
            substate_value: crate::substate_decode::indexer_substate_json(&SubstateValue::Utxo(utxo)),
        }];

        let intent = StealthTransferIntent {
            from_account: from_component(),
            resource_address: tari_resource(),
            fee: BoundaryAmount::new(2000),
            inputs: vec![StealthInputSpec {
                commitment: crate::types::stealth::CommitmentBytes::from_hex(&commitment_hex).unwrap(),
                owner_account_pk: owner_pk,
            }],
            outputs: vec![output_spec(value)],
            revealed_input_amount: 0,
            revealed_output_amount: 0,
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
            pay_fee_from_revealed: false,
        };
        let spend_secrets = vec![SecretKeyBytes::from_bytes(view_secret.as_bytes()).unwrap()];

        // Seed the resolver + want list. The live want set carries the from-account component + vault
        // wants plus exactly one stealth UTXO want.
        let (partial, want_list) =
            build_stealth_unsigned_with_wants_with_seed(Network::LocalNet, &intent, &seed).unwrap();
        assert_eq!(
            want_list
                .0
                .iter()
                .filter(|w| matches!(w, crate::inputs::WantItem::StealthUtxo { .. }))
                .count(),
            1,
            "one stealth UTXO want"
        );

        // Drive the host fetch loop to completion: round 0 starts empty (exercising NeedMore), each
        // later round serves the requested ids from the from-account store (component + vault) and the
        // supplied UTXO. The first NeedMore must surface the discovered UTXO id.
        let mut partial_opt = Some(partial);
        let mut to_serve: Vec<String> = Vec::new();
        let mut saw_utxo_in_fetch = false;
        let mut round = 0;
        let assembled = loop {
            let p = partial_opt.take().unwrap();
            let batch: Vec<FetchedSubstate> = to_serve
                .iter()
                .filter_map(|id| {
                    account_substate_for(id).or_else(|| (id == &substate_id.to_string()).then(|| fetched[0].clone()))
                })
                .collect();
            match apply_fetched_substates_stealth(p, Network::LocalNet, &batch, &spend_secrets, &test_keys()).unwrap() {
                StealthResolution::Resolved(a) => break *a,
                StealthResolution::NeedMore { partial, fetch_ids } => {
                    if fetch_ids.iter().any(|id| id == &substate_id.to_string()) {
                        saw_utxo_in_fetch = true;
                    }
                    to_serve = fetch_ids;
                    partial_opt = Some(*partial);
                },
            }
            round += 1;
            assert!(round < 6, "the loop must converge");
        };
        assert!(saw_utxo_in_fetch, "NeedMore must surface the discovered UTXO id");

        // The looped product matches the one-shot path for the same inputs (deterministic statement
        // fields — the proofs are non-byte-stable, so compare the structural statement).
        let one_shot = build_stealth_transfer_unsigned_with_seed(
            Network::LocalNet,
            &intent,
            &fetched,
            &spend_secrets,
            &test_keys(),
            &seed,
        )
        .unwrap();

        let looped_stmt = transfer_statement(assembled.unsigned());
        let one_shot_stmt = transfer_statement(one_shot.unsigned());
        assert_eq!(looped_stmt.inputs_statement, one_shot_stmt.inputs_statement);
        assert_eq!(
            looped_stmt.outputs_statement.outputs,
            one_shot_stmt.outputs_statement.outputs
        );
        assert!(looped_stmt.balance_proof.is_some());
        assert_eq!(
            assembled
                .signature_requirements()
                .seal_signer
                .as_ref()
                .map(|s| &s.account_pk_hex),
            one_shot
                .signature_requirements()
                .seal_signer
                .as_ref()
                .map(|s| &s.account_pk_hex),
        );
        // The resolved UTXO substate is attached as a transaction input on the looped path too.
        assert!(
            assembled
                .unsigned()
                .inputs()
                .iter()
                .any(|i| i.substate_id().to_string() == substate_id.to_string())
        );
        // The from-account component + its vault are declared inputs (resolved via the account wants).
        let input_ids: Vec<String> = assembled
            .unsigned()
            .inputs()
            .iter()
            .map(|i| i.substate_id().to_string())
            .collect();
        assert!(
            input_ids.contains(&from_component().as_str().to_string()),
            "from-account component declared"
        );
        let vault_id = tari_engine_types::substate::SubstateId::Vault(from_vault_id()).to_string();
        assert!(input_ids.contains(&vault_id), "from-account vault declared");
    }

    /// A missing required stealth UTXO surfaces a clean `INVALID` ("not found") error through the
    /// looped path once the resolver marks the id definitively absent (the host fetched it and the
    /// indexer returned nothing), never a panic or an infinite loop.
    #[test]
    fn two_phase_loop_missing_required_utxo_errors() {
        let value = 1_000_000u64;
        let seed = seed_for(100);

        // A stealth input whose UTXO is never supplied.
        let input_mask = RistrettoSecretKey::from_canonical_bytes(secret(180).as_bytes()).unwrap();
        let commitment = tari_engine_types::crypto::commit_u64_amount(&input_mask, value).to_byte_type();
        let commitment_hex = hex::encode(commitment.as_bytes());

        let intent = StealthTransferIntent {
            from_account: from_component(),
            resource_address: tari_resource(),
            fee: BoundaryAmount::new(2000),
            inputs: vec![StealthInputSpec {
                commitment: crate::types::stealth::CommitmentBytes::from_hex(&commitment_hex).unwrap(),
                owner_account_pk: pk_bytes(9),
            }],
            outputs: vec![output_spec(value)],
            revealed_input_amount: 0,
            revealed_output_amount: 0,
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
            pay_fee_from_revealed: false,
        };
        let spend_secrets = vec![secret(181)];

        let (partial, _wants) = build_stealth_unsigned_with_wants_with_seed(Network::LocalNet, &intent, &seed).unwrap();

        // Drive the loop serving only the from-account component + vault (never the UTXO). Once the
        // account resolves, the still-missing UTXO is marked definitively absent and errors `INVALID`.
        let mut partial_opt = Some(partial);
        let mut to_serve: Vec<String> = Vec::new();
        let mut round = 0;
        let err = loop {
            let p = partial_opt.take().unwrap();
            let batch: Vec<FetchedSubstate> = to_serve.iter().filter_map(|id| account_substate_for(id)).collect();
            match apply_fetched_substates_stealth(p, Network::LocalNet, &batch, &spend_secrets, &test_keys()) {
                Ok(StealthResolution::Resolved(_)) => panic!("must not resolve without the UTXO"),
                Ok(StealthResolution::NeedMore { partial, fetch_ids }) => {
                    to_serve = fetch_ids;
                    partial_opt = Some(*partial);
                },
                Err(e) => break e,
            }
            round += 1;
            assert!(round < 6, "the loop must terminate with an error");
        };
        assert_eq!(err.code(), "INVALID");
    }

    // ---- Revealed-output deposit fold ------------------------------------------------------------

    /// One stealth output with a per-output blinded `amount` + revealed `revealed_amount` slice,
    /// addressed to `account_seed`/`view_seed`.
    fn revealed_output_spec(account_seed: u8, view_seed: u8, amount: u64, revealed_amount: u64) -> StealthOutputSpec {
        StealthOutputSpec {
            destination_account_pk: pk_bytes(account_seed),
            destination_view_pk: pk_bytes(view_seed),
            amount,
            revealed_amount,
            resource_address: tari_resource(),
            resource_view_key: None,
            memo: None,
            pay_to: StealthPayTo::StealthPublicKey,
            utxo_tag: None,
            minimum_value_promise: 0,
        }
    }

    fn create_account_owners(unsigned: &UnsignedTransaction) -> Vec<RistrettoPublicKey> {
        use tari_crypto::tari_utilities::ByteArray;
        unsigned
            .instructions()
            .iter()
            .filter_map(|i| match i {
                Instruction::CreateAccount { owner_public_key, .. } => {
                    Some(RistrettoPublicKey::from_canonical_bytes(owner_public_key.as_bytes()).unwrap())
                },
                _ => None,
            })
            .collect()
    }

    fn count_take_from_bucket(unsigned: &UnsignedTransaction) -> usize {
        unsigned
            .instructions()
            .iter()
            .filter(|i| matches!(i, Instruction::TakeFromBucket { .. }))
            .count()
    }

    // (b) A single revealed output deposits the whole `output_bucket` via one CreateAccount (no
    //     TakeFromBucket split). Balance: revealed_input == stealth_out + revealed_out.
    #[test]
    fn single_revealed_output_deposits_whole_bucket() {
        let entropy = entropy_for(1, 110);
        let intent = StealthTransferIntent {
            from_account: from_component(),
            resource_address: tari_resource(),
            fee: BoundaryAmount::new(2000),
            inputs: Vec::<StealthInputSpec>::new(),
            // revealed_input 1_500_000 = stealth output 1_000_000 + revealed output 500_000.
            outputs: vec![revealed_output_spec(3, 4, 1_000_000, 500_000)],
            revealed_input_amount: 1_500_000,
            revealed_output_amount: 500_000,
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
            pay_fee_from_revealed: false,
        };
        let partial = assemble_from_intent(&intent, &entropy).unwrap();
        let unsigned = partial.unsigned();

        // The revealed output bucket is captured on the workspace twice: once for the revealed-input
        // withdraw, once for the stealth-transfer output. At least one PutLastInstructionOutputOnWorkspace.
        assert!(
            unsigned
                .instructions()
                .iter()
                .any(|i| matches!(i, Instruction::PutLastInstructionOutputOnWorkspace { .. }))
        );
        // Single output ⇒ no split.
        assert_eq!(count_take_from_bucket(unsigned), 0, "single output ⇒ no TakeFromBucket");
        // Exactly one CreateAccount, keyed by the destination account pk.
        let owners = create_account_owners(unsigned);
        assert_eq!(owners.len(), 1, "single revealed output ⇒ one CreateAccount");
        let expected = RistrettoPublicKey::from_canonical_bytes(pk_bytes(3).as_bytes()).unwrap();
        assert_eq!(owners[0], expected, "deposit keyed by destination_account_pk");
    }

    // (c) A multi-output revealed split emits one TakeFromBucket + one CreateAccount per output,
    //     keyed by each output's destination, in index order.
    #[test]
    fn multi_revealed_output_splits_bucket_per_output() {
        let entropy = entropy_for(2, 120);
        let intent = StealthTransferIntent {
            from_account: from_component(),
            resource_address: tari_resource(),
            fee: BoundaryAmount::new(2000),
            inputs: Vec::<StealthInputSpec>::new(),
            // revealed_input 2_500_000 = stealth out (500_000 + 500_000) + revealed out (1_000_000 + 500_000).
            outputs: vec![
                revealed_output_spec(3, 4, 500_000, 1_000_000),
                revealed_output_spec(5, 6, 500_000, 500_000),
            ],
            revealed_input_amount: 2_500_000,
            revealed_output_amount: 1_500_000,
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
            pay_fee_from_revealed: false,
        };
        let partial = assemble_from_intent(&intent, &entropy).unwrap();
        let unsigned = partial.unsigned();

        // Two outputs ⇒ two splits + two CreateAccount deposits.
        assert_eq!(
            count_take_from_bucket(unsigned),
            2,
            "two revealed outputs ⇒ two TakeFromBucket"
        );
        let owners = create_account_owners(unsigned);
        assert_eq!(owners.len(), 2, "two revealed outputs ⇒ two CreateAccount");
        assert_eq!(
            owners[0],
            RistrettoPublicKey::from_canonical_bytes(pk_bytes(3).as_bytes()).unwrap()
        );
        assert_eq!(
            owners[1],
            RistrettoPublicKey::from_canonical_bytes(pk_bytes(5).as_bytes()).unwrap()
        );
    }

    // (d) No revealed output ⇒ no deposit fold at all (no CreateAccount).
    #[test]
    fn no_revealed_output_emits_no_deposit() {
        let intent = balanced_intent(1_000_000, 1_000_000);
        let entropy = entropy_for(1, 130);
        let partial = assemble_from_intent(&intent, &entropy).unwrap();
        let owners = create_account_owners(partial.unsigned());
        assert!(owners.is_empty(), "no revealed output ⇒ no CreateAccount deposit");
    }

    // (e) A mismatched per-output revealed sum is rejected before assembly with a VALIDATION error.
    #[test]
    fn mismatched_revealed_sum_is_validation_error() {
        let entropy = entropy_for(1, 140);
        let intent = StealthTransferIntent {
            from_account: from_component(),
            resource_address: tari_resource(),
            fee: BoundaryAmount::new(2000),
            inputs: Vec::<StealthInputSpec>::new(),
            // Per-output revealed sum (400_000) != revealed_output_amount (500_000).
            outputs: vec![revealed_output_spec(3, 4, 1_000_000, 400_000)],
            revealed_input_amount: 1_500_000,
            revealed_output_amount: 500_000,
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
            pay_fee_from_revealed: false,
        };
        let err = assemble_from_intent(&intent, &entropy).unwrap_err();
        assert_eq!(err.code(), "VALIDATION");
    }
}
