//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Two-round, pure/sync input resolution for a public transfer.
//!
//! A host builds a public transfer without an explicit input set: the core derives the substates
//! that must be fetched (the [`WantList`]), the host performs the (only) I/O of fetching them, and the
//! core folds the results back into a complete unsigned transaction. There are no callbacks into the
//! host and no networking/clock/RNG in the core — the host drives a simple loop:
//!
//! ```text
//! (partial, wants) = build_public_transfer_unsigned_with_wants(network, intent)
//! ids = wants.seed_substate_ids()           // concrete ids to fetch first
//! loop {
//!     fetched = host.fetch(ids)              // the host's only I/O
//!     match apply_fetched_substates(partial, fetched) {
//!         Resolved(partial)        => break,           // ready to seal
//!         NeedMore { fetch_ids, .. } => ids = fetch_ids, // realistic depth: 1–2 round-trips
//!     }
//! }
//! ```
//!
//! ## Algorithm
//!
//! A fixed-point resolution loop with the I/O lifted out to the host. The public-transfer want set is
//! four items: the from-component and its from-vault (both `required`),
//! the recipient component, and the recipient vault (both optional — the `create_account_with_bucket`
//! instruction creates them if absent).
//!
//! ## Input ordering
//!
//! Resolved inputs are accumulated into a `Vec` in a deterministic walk order (wants in list order;
//! vaults in component-state order). [`PartialTransaction`] does not fold them into the unsigned tx's
//! input set until `Resolved`, and even then preserves that order. The seal path is responsible for
//! canonical sorting before the bytes are hashed.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tari_engine_types::{
    component::derive_component_address_from_public_key,
    substate::{SubstateId, SubstateValue},
};
use tari_ootle_common_types::SubstateRequirement;
use tari_ootle_transaction::UnsignedTransaction;
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib_types::{ComponentAddress, ResourceAddress, constants::TARI_TOKEN};

use crate::{
    builder::{apply_epoch_and_dry_run, build_transfer_recipe_builder, resolve_transfer_recipe},
    types::{error::OotleSdkError, intent::PublicTransferIntent, network::Network},
};

/// One substate (or vault) the host must fetch for the core to fold in. Addresses are boundary
/// strings (`component_<hex>` / `resource_<hex>` / `vault_<hex>`); no internal `tari_*` types leak
/// across this record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WantItem {
    /// The vault holding `resource_address` inside `component_address`. The resolver fetches the
    /// component, reads its state for vault ids, then fetches those vaults to find the match.
    VaultForResource {
        /// The component whose vaults are searched.
        component_address: String,
        /// The resource the vault must hold.
        resource_address: String,
        /// When `true`, a missing component/vault is an error; when `false`, it is silently dropped.
        required: bool,
    },
    /// A specific substate (added as an input if it exists / unconditionally if `required`).
    SpecificSubstate {
        /// The canonical substate id (`component_<hex>`, `resource_<hex>`, …).
        substate_id: String,
        /// When `true`, the input is added without verifying the substate exists.
        required: bool,
    },
    /// Fetch the component state and add **all** vaults found in it as inputs.
    AllComponentVaults {
        /// The component whose vaults are all added.
        component_address: String,
    },
    /// A stealth UTXO input to spend. Resolves to the `UtxoAddress` substate id derived from
    /// `resource_address` + `commitment`; the host fetches it like any other substate. Once fetched,
    /// the resolver validates the UTXO (not frozen/burnt), recovers the spend mask by decrypting it
    /// with the caller-supplied view-only secret (keyed by `owner_account_pk`), and classifies the
    /// signer.
    StealthUtxo {
        /// Canonical `resource_<hex>` string of the resource the UTXO holds.
        resource_address: String,
        /// The on-chain Pedersen commitment in lowercase hex (32 bytes = 64 hex chars).
        commitment: String,
        /// Owner account public key in lowercase hex — keys the spend-secret lookup at decrypt time.
        owner_account_pk: String,
        /// Always `true` for stealth inputs — a missing UTXO is an error.
        required: bool,
    },
}

impl WantItem {
    /// The substate ids the host should fetch *first* to make progress on this want. (The resolver
    /// may discover further ids — e.g. vault ids inside a component — on a later round.)
    pub fn seed_substate_ids(&self) -> Vec<String> {
        match self {
            WantItem::VaultForResource { component_address, .. } => vec![component_address.clone()],
            WantItem::SpecificSubstate { substate_id, .. } => vec![substate_id.clone()],
            WantItem::AllComponentVaults { component_address } => vec![component_address.clone()],
            WantItem::StealthUtxo {
                resource_address,
                commitment,
                ..
            } => crate::stealth::inputs::stealth_utxo_substate_id(resource_address, commitment)
                .map(|id| vec![id.to_string()])
                .unwrap_or_default(),
        }
    }
}

/// The flat list of [`WantItem`]s the host must fetch before the next `apply` call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WantList(pub Vec<WantItem>);

impl WantList {
    /// The deduplicated set of substate ids the host should fetch to satisfy this whole list.
    pub fn seed_substate_ids(&self) -> Vec<String> {
        let mut out = Vec::new();
        for want in &self.0 {
            for id in want.seed_substate_ids() {
                if !out.contains(&id) {
                    out.push(id);
                }
            }
        }
        out
    }
}

/// One substate the host fetched from the indexer and is handing back to the core.
///
/// `substate_value` is the **neutral carrier**: the host passes the indexer's substate JSON straight
/// through (`serde_json::Value`), and the core parses it into the internal `SubstateValue`. A parse
/// failure ⇒ [`OotleSdkError::Parse`]. (`serde_json::Value` is the simplest carrier — it avoids a
/// second string-escaping layer and round-trips the indexer shape exactly.)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FetchedSubstate {
    /// The canonical substate id this value is for.
    pub substate_id: String,
    /// The substate version the indexer returned (carried for the host; the resolver folds
    /// unversioned inputs).
    pub version: u64,
    /// The indexer's `SubstateValue` JSON, passed through verbatim.
    pub substate_value: serde_json::Value,
}

/// The result of folding a fetched batch into a [`PartialTransaction`].
///
/// Both arms own the (updated) partial: `apply_fetched_substates` consumes the partial and hands it
/// back so the host can either seal it (`Resolved`) or feed the next fetched batch into it
/// (`NeedMore`).
#[derive(Debug)]
pub enum Resolution {
    /// All required wants are satisfied; the partial is ready to seal.
    Resolved(PartialTransaction),
    /// More substates are needed — fetch `fetch_ids` and call [`apply_fetched_substates`] again on
    /// the returned `partial`.
    NeedMore {
        /// The still-in-progress partial, carrying the cache + resolved inputs accumulated so far.
        partial: PartialTransaction,
        /// The wants still outstanding (the *semantic* request — e.g. "the vault for resource X in
        /// component Y"). Informational; the host fetches `fetch_ids`, not this.
        want_list: WantList,
        /// The **concrete** substate ids the host must fetch next — the authoritative next-fetch set.
        ///
        /// Unlike `want_list.seed_substate_ids()` (which only names a want's seed, e.g. a component
        /// address), this carries the ids the resolver actually discovered it still needs, including
        /// vault ids read out of a component on a prior round. A host that fetches `want_list` seeds
        /// alone could never learn the discovered vault id and would loop forever; fetching
        /// `fetch_ids` converges. Lowercase-hex canonical substate ids.
        fetch_ids: Vec<String>,
    },
}

/// One classified stealth signer accumulated during input resolution, carried as boundary bytes
/// until the final `SignatureRequirements` + signing instructions are assembled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StealthSignerEntry {
    /// The owner account public key (lowercase hex) that must authorize spending this input.
    pub account_pk_hex: String,
    /// The sender public nonce recovered from the UTXO output body.
    pub public_nonce_bytes: crate::types::bytes::PublicKeyBytes,
}

/// The opaque, in-progress public transfer.
///
/// Holds the unsigned transaction being assembled, the wants still outstanding, the substate cache
/// (so a later round need not re-fetch), and the resolved inputs accumulated in a stable `Vec`
/// (not a set) so the seal path can canonicalize/sort them before sealing. This is a handle, not a
/// wire record: it deliberately does not derive `Serialize`.
///
/// **Stealth state:** when a [`WantItem::StealthUtxo`] is resolved, the recovered witness, the
/// aggregate input mask, and the evolving signer classification are accumulated here too. These
/// fields are zero/empty for a plain public transfer.
///
/// **Secret hygiene:** the two secret-bearing stealth fields — `agg_input_mask` and the per-input
/// masks inside `stealth_input_witnesses` — are `RistrettoSecretKey` scalars, which are
/// `ZeroizeOnDrop`, so they are wiped from memory when this `PartialTransaction` drops; no extra
/// wrapper is needed. Caller-supplied spend secrets are never stored here (they are borrowed for the
/// resolve call only) and the `SecretKeyBytes` newtype they are parsed into zeroizes on its own drop.
#[derive(Debug)]
pub struct PartialTransaction {
    unsigned: UnsignedTransaction,
    wants: Vec<WantItem>,
    /// `Some(None)` = fetched-but-absent (a definitive "not found"); absent key = never fetched.
    cache: HashMap<SubstateId, Option<SubstateValue>>,
    /// The substate ids the previous round asked the host to fetch. On the next `apply`, any of these
    /// not present in the returned batch is recorded as absent (`Some(None)`). Without this, an
    /// optional substate the indexer reports missing would be re-requested forever.
    pending_fetch: Vec<SubstateId>,
    resolved: Vec<SubstateRequirement>,

    // --- stealth accumulation state ---------------------------------------------------------------
    /// Recovered per-input witnesses (mask + value), folded into the transfer statement / balance
    /// proof at assembly time.
    ///
    /// **Privacy:** each witness's only secret is its `MaskAndValue::mask`, a
    /// `tari_crypto::ristretto::RistrettoSecretKey`, which derives `ZeroizeOnDrop`. So when this `Vec`
    /// drops (with the `PartialTransaction`), every recovered mask is wiped from memory — the wipe
    /// happens transitively through its `RistrettoSecretKey` field, so no wrapper is needed here.
    stealth_input_witnesses: Vec<tari_ootle_wallet_crypto::StealthInputWitness>,
    /// The running sum of per-input spend masks. Stored as an opaque secret scalar; needed for the
    /// balance proof. **Privacy:** the spend secrets used to recover it are not stored, and the mask
    /// itself is zeroized on drop — `RistrettoSecretKey` derives `ZeroizeOnDrop`, so this scalar is
    /// wiped from memory when the `PartialTransaction` drops.
    agg_input_mask: tari_crypto::ristretto::RistrettoSecretKey,
    /// Whether the account key must sign (set from `intent.revealed_input_amount > 0`). Drives the
    /// seal-vs-required signer split.
    must_sign_with_account_key: bool,
    /// The seal signer — the first non-account-key-signed stealth input.
    seal_signer: Option<StealthSignerEntry>,
    /// All other required signers, in resolution order.
    required_signers: Vec<StealthSignerEntry>,
    /// The stealth build context (intent + pinned entropy) the host-driven loop carries so the final
    /// `apply` can assemble the transfer statement + balance proof without the host re-passing them.
    /// `None` for a public transfer.
    stealth_ctx: Option<crate::stealth::assemble::StealthBuildCtx>,
}

impl PartialTransaction {
    /// The resolved [`UnsignedTransaction`] with every resolved input folded in (the seal path seals
    /// this).
    ///
    /// Inputs are folded in the deterministic accumulation order; the seal path owns canonical
    /// sorting.
    pub fn into_unsigned(self) -> UnsignedTransaction {
        let mut unsigned = self.unsigned;
        for req in self.resolved {
            // De-dup against anything already present (e.g. explicit inputs).
            if !unsigned.inputs().contains(&req) {
                unsigned.add_input(req);
            }
        }
        unsigned
    }

    /// Like [`Self::into_unsigned`] but **borrows** — produces the resolved [`UnsignedTransaction`]
    /// by cloning, without consuming the partial. Used by the co-sign path so party A can derive the
    /// unsigned record to ship to B and keep the partial to seal it (the seal path re-derives the
    /// identical value via [`Self::into_unsigned`] + the canonical sort, so A and B agree).
    pub fn cloned_unsigned(&self) -> UnsignedTransaction {
        let mut unsigned = self.unsigned.clone();
        for req in &self.resolved {
            if !unsigned.inputs().contains(req) {
                unsigned.add_input(req.clone());
            }
        }
        unsigned
    }

    /// Borrows the resolved inputs accumulated so far (deterministic order). Mainly for tests / the
    /// seal path.
    pub fn resolved_inputs(&self) -> &[SubstateRequirement] {
        &self.resolved
    }

    /// Borrows the wants still outstanding.
    pub fn outstanding_wants(&self) -> &[WantItem] {
        &self.wants
    }

    /// Builds an in-progress partial seeded with stealth-UTXO wants.
    ///
    /// `must_sign_with_account_key` comes from `intent.revealed_input_amount > 0 ||
    /// intent.pay_fee_from_revealed` (either draws on the account, so the account key must seal). The `unsigned` is
    /// the half-built transaction finished at assembly time; the wants are
    /// [`WantList::from_stealth_inputs`]. The host drives the fetch loop calling
    /// [`apply_fetched_substates_with_secrets`] with the caller's view-only spend secrets so the
    /// stealth branch can decrypt the recovered masks; `ctx` (intent + pinned entropy) is stashed so
    /// the final apply can assemble the transfer statement without the host re-passing them.
    pub fn new_for_stealth_inputs(
        unsigned: UnsignedTransaction,
        intent: &crate::types::stealth::StealthTransferIntent,
        ctx: crate::stealth::assemble::StealthBuildCtx,
    ) -> Self {
        Self::new_for_stealth_inputs_with_wants(unsigned, intent, ctx, WantList::from_stealth_inputs(intent))
    }

    /// Like [`Self::new_for_stealth_inputs`] but seeds the resolver with an explicit want list (the
    /// live two-phase driver passes [`WantList::from_stealth_inputs_with_account`] so the from-account
    /// component + vault resolve and are declared as inputs; the one-shot path passes the bare
    /// stealth-UTXO wants). `pending_fetch` stays empty for the same reason as
    /// [`Self::new_for_stealth_inputs`] — the resolver schedules each unfetched want itself.
    pub fn new_for_stealth_inputs_with_wants(
        unsigned: UnsignedTransaction,
        intent: &crate::types::stealth::StealthTransferIntent,
        ctx: crate::stealth::assemble::StealthBuildCtx,
        wants: WantList,
    ) -> Self {
        Self {
            unsigned,
            wants: wants.0,
            cache: HashMap::new(),
            // No pre-seeded `pending_fetch`: at seed time the host has fetched nothing, and every
            // stealth UTXO want is required. The first `apply` over a batch that omits a wanted UTXO
            // must report `NeedMore { fetch_ids }` (so the host can fetch it next round), not mark it
            // definitively-absent and error. The resolver schedules each unfetched want into
            // `pending_fetch` itself as it discovers them, so multi-round convergence works.
            pending_fetch: Vec::new(),
            resolved: Vec::new(),
            stealth_input_witnesses: Vec::new(),
            agg_input_mask: tari_crypto::ristretto::RistrettoSecretKey::default(),
            must_sign_with_account_key: intent.revealed_input_amount > 0 || intent.pay_fee_from_revealed,
            seal_signer: None,
            required_signers: Vec::new(),
            stealth_ctx: Some(ctx),
        }
    }

    /// Takes the stashed stealth build context out of the resolver (used by the final stealth `apply`
    /// to assemble). `None` for a public transfer or once already taken.
    pub fn take_stealth_ctx(&mut self) -> Option<crate::stealth::assemble::StealthBuildCtx> {
        self.stealth_ctx.take()
    }

    /// Re-stashes the stealth build context into the resolver (used by a non-final stealth `apply`
    /// round to keep the intent + entropy available for the next round's assembly).
    pub fn restore_stealth_ctx(&mut self, ctx: crate::stealth::assemble::StealthBuildCtx) {
        self.stealth_ctx = Some(ctx);
    }

    /// Builds an in-progress partial seeded with the given want list (the resolved/want-list path).
    ///
    /// The wants' seed ids are recorded into `pending_fetch` so the first `apply` can mark any the
    /// indexer omits as definitively absent. No stealth state is carried (this is a public/generic
    /// resolution).
    pub fn new_for_wants(unsigned: UnsignedTransaction, wants: WantList) -> Self {
        Self::new_for_wants_with_extra_inputs(unsigned, wants, Vec::new())
    }

    /// Like [`Self::new_for_wants`] but pre-seeds the resolved set with `extra_inputs` — inputs the
    /// caller pins in addition to whatever resolution derives (e.g. a template's internally-referenced
    /// resource). They are folded into the final inputs alongside the resolved ones; the resolvers'
    /// `push_unique` de-dups any that resolution would also add.
    pub fn new_for_wants_with_extra_inputs(
        unsigned: UnsignedTransaction,
        wants: WantList,
        extra_inputs: Vec<SubstateRequirement>,
    ) -> Self {
        Self {
            unsigned,
            wants: wants.0.clone(),
            cache: HashMap::new(),
            pending_fetch: wants
                .seed_substate_ids()
                .iter()
                .filter_map(|s| SubstateId::from_str_checked(s).ok())
                .collect(),
            resolved: extra_inputs,
            stealth_input_witnesses: Vec::new(),
            agg_input_mask: tari_crypto::ristretto::RistrettoSecretKey::default(),
            must_sign_with_account_key: false,
            seal_signer: None,
            required_signers: Vec::new(),
            stealth_ctx: None,
        }
    }

    /// Builds a fully-resolved partial from an already-complete explicit input set (the
    /// explicit-input short-circuit shared by the public and generic front-ends). Public so the
    /// generic builder reuses the identical short-circuit.
    pub fn new_with_explicit_inputs(unsigned: UnsignedTransaction, resolved: Vec<SubstateRequirement>) -> Self {
        Self::with_explicit_inputs(unsigned, resolved)
    }

    /// Folds an already-complete input set in directly (the explicit short-circuit path) and marks
    /// the partial fully resolved.
    fn with_explicit_inputs(unsigned: UnsignedTransaction, resolved: Vec<SubstateRequirement>) -> Self {
        Self {
            unsigned,
            wants: Vec::new(),
            cache: HashMap::new(),
            pending_fetch: Vec::new(),
            resolved,
            stealth_input_witnesses: Vec::new(),
            agg_input_mask: tari_crypto::ristretto::RistrettoSecretKey::default(),
            must_sign_with_account_key: false,
            seal_signer: None,
            required_signers: Vec::new(),
            stealth_ctx: None,
        }
    }

    /// Borrows the stealth input witnesses recovered so far (folded into the transfer statement /
    /// balance proof at assembly time).
    pub fn stealth_input_witnesses(&self) -> &[tari_ootle_wallet_crypto::StealthInputWitness] {
        &self.stealth_input_witnesses
    }

    /// Borrows the aggregate stealth input mask accumulated so far (needed for the balance proof).
    pub fn agg_input_mask(&self) -> &tari_crypto::ristretto::RistrettoSecretKey {
        &self.agg_input_mask
    }

    /// Whether the account key must sign the stealth transfer.
    pub fn must_sign_with_account_key(&self) -> bool {
        self.must_sign_with_account_key
    }

    /// The classified seal signer, if any.
    pub fn seal_signer(&self) -> Option<&StealthSignerEntry> {
        self.seal_signer.as_ref()
    }

    /// The classified required signers, in resolution order.
    pub fn required_signers(&self) -> &[StealthSignerEntry] {
        &self.required_signers
    }
}

/// Round 1: build the public-transfer unsigned tx **and** the want list to resolve it.
///
/// Emits the same instruction recipe as [`crate::builder::build_public_transfer_unsigned`], but
/// attaches no explicit inputs — instead it derives the 3-item public-transfer want set, deriving the
/// recipient component address from the recipient public key via
/// `derive_component_address_from_public_key`.
///
/// **Explicit-input short-circuit:** if `intent.inputs` is non-empty the caller has chosen the
/// explicit path, so this returns a `PartialTransaction` already carrying those inputs and an empty
/// want list (the host's first `apply_fetched_substates` with an empty batch yields `Resolved`),
/// keeping the entry point usable in both modes.
pub fn build_public_transfer_unsigned_with_wants(
    network: Network,
    intent: &PublicTransferIntent,
) -> Result<(PartialTransaction, WantList), OotleSdkError> {
    let recipe = resolve_transfer_recipe(intent)?;
    let builder = build_transfer_recipe_builder(network, &recipe, tari_ootle_common_types::Epoch(intent.max_epoch));
    let builder = apply_epoch_and_dry_run(builder, intent);
    let unsigned = builder.build_unsigned();

    // Explicit-input short-circuit: caller supplied an input set, so skip resolution.
    if !intent.inputs.is_empty() {
        let inputs = intent.inputs_to_internal()?;
        let partial = PartialTransaction::with_explicit_inputs(unsigned, inputs);
        return Ok((partial, WantList(Vec::new())));
    }

    // The recipient account component is derived from its public key (the intent carries the pk).
    let recipient_component = derive_component_address_from_public_key(&ACCOUNT_TEMPLATE_ADDRESS, &recipe.recipient_pk);

    let wants = public_transfer_want_set(recipe.from_component, recipient_component, recipe.resource);
    let partial = PartialTransaction::new_for_wants(unsigned, wants.clone());
    Ok((partial, wants))
}

/// The public-transfer want set.
fn public_transfer_want_set(
    from_component: ComponentAddress,
    recipient_component: ComponentAddress,
    resource: ResourceAddress,
) -> WantList {
    WantList(vec![
        // The from-component itself — the fee + withdraw instructions call it, so it must be a
        // declared input (this path uses no auto-fill).
        WantItem::SpecificSubstate {
            substate_id: from_component.to_string(),
            required: true,
        },
        // The from-vault we pay from — must exist.
        WantItem::VaultForResource {
            component_address: from_component.to_string(),
            resource_address: resource.to_string(),
            required: true,
        },
        // The recipient account component, if it already exists.
        WantItem::SpecificSubstate {
            substate_id: recipient_component.to_string(),
            required: false,
        },
        // The recipient vault to deposit into, if it exists (else CreateAccount creates it).
        WantItem::VaultForResource {
            component_address: recipient_component.to_string(),
            resource_address: resource.to_string(),
            required: false,
        },
    ])
}

/// Round 2: fold a fetched batch into the partial and report whether resolution is complete.
///
/// Parses every [`FetchedSubstate`] into the internal `SubstateValue` (caching it; a malformed value
/// ⇒ [`OotleSdkError::Parse`]), runs one pass of the resolve-loop body over the outstanding wants
/// using the now-larger cache, then:
/// - if **all required** wants are satisfied ⇒ folds resolved inputs in and returns [`Resolution::Resolved`];
/// - else returns [`Resolution::NeedMore`] with the remaining wants;
/// - if a pass makes **no progress and schedules no new fetch** (the deadlock guard) ⇒ [`OotleSdkError::Resolution`].
pub fn apply_fetched_substates(
    partial: PartialTransaction,
    fetched: &[FetchedSubstate],
) -> Result<Resolution, OotleSdkError> {
    // The public-transfer path carries no stealth wants, so an empty secrets map suffices.
    apply_fetched_substates_with_secrets(partial, fetched, &HashMap::new())
}

/// Round 2 with caller-supplied stealth spend secrets.
///
/// Identical to [`apply_fetched_substates`] for the public-transfer wants, but additionally resolves
/// any [`WantItem::StealthUtxo`] in the same pass: each fetched UTXO is validated, its mask recovered
/// by decrypting with the matching view-only secret from `spend_secrets` (keyed by owner account pk
/// hex), and the witness/mask/signer-classification accumulated into the partial. The stealth branch
/// lives inside the resolve loop so the deadlock guard and pending-fetch tracking cover it (a missing
/// required UTXO surfaces as an error, never an infinite loop).
///
/// `spend_secrets` is borrowed for the call only — it is never stored in the partial.
pub fn apply_fetched_substates_with_secrets(
    mut partial: PartialTransaction,
    fetched: &[FetchedSubstate],
    spend_secrets: &HashMap<String, crate::types::bytes::SecretKeyBytes>,
) -> Result<Resolution, OotleSdkError> {
    // 1. Parse + cache the fetched batch (host's I/O result → internal types).
    for f in fetched {
        let id = SubstateId::from_str_checked(&f.substate_id)?;
        let value: SubstateValue = serde_json::from_value(f.substate_value.clone())
            .map_err(|e| OotleSdkError::Parse(format!("invalid substate value for '{}': {e}", f.substate_id)))?;
        partial.cache.insert(id, Some(value));
    }

    // 2. Any id we asked the host to fetch that did not come back is definitively absent — record it as `Some(None)`.
    //    Without this, an optional substate the indexer omits would be re-requested forever.
    let pending = std::mem::take(&mut partial.pending_fetch);
    for id in pending {
        partial.cache.entry(id).or_insert(None);
    }

    // 3. One pass over the outstanding wants using the now-larger cache. The stealth accumulation state is taken out as
    //    locals so the per-want call can borrow the cache and each stealth field mutably without conflicting with
    //    `partial`; it is written back below.
    let wants = std::mem::take(&mut partial.wants);
    let must_sign_with_account_key = partial.must_sign_with_account_key;
    let mut stealth_witnesses = std::mem::take(&mut partial.stealth_input_witnesses);
    let mut agg_input_mask = std::mem::take(&mut partial.agg_input_mask);
    let mut seal_signer = partial.seal_signer.take();
    let mut required_signers = std::mem::take(&mut partial.required_signers);

    let mut still_outstanding: Vec<WantItem> = Vec::new();
    let mut to_fetch: Vec<SubstateId> = Vec::new();
    let mut progress = false;

    for want in wants {
        let mut want_fetch: Vec<SubstateId> = Vec::new();
        let satisfied = match &want {
            WantItem::StealthUtxo {
                resource_address,
                commitment,
                owner_account_pk,
                ..
            } => crate::stealth::inputs::resolve_one_stealth_utxo(
                resource_address,
                commitment,
                owner_account_pk,
                &partial.cache,
                spend_secrets,
                &mut stealth_witnesses,
                &mut agg_input_mask,
                must_sign_with_account_key,
                &mut seal_signer,
                &mut required_signers,
                &mut partial.resolved,
                &mut want_fetch,
            )?,
            _ => resolve_one(&want, &partial.cache, &mut partial.resolved, &mut want_fetch)?,
        };
        if satisfied {
            progress = true;
        } else {
            to_fetch.append(&mut want_fetch);
            still_outstanding.push(want);
        }
    }

    // Write the stealth accumulation state back into the partial.
    partial.stealth_input_witnesses = stealth_witnesses;
    partial.agg_input_mask = agg_input_mask;
    partial.seal_signer = seal_signer;
    partial.required_signers = required_signers;
    partial.wants = still_outstanding;

    // 4. All wants resolved ⇒ Resolved.
    if partial.wants.is_empty() {
        return Ok(Resolution::Resolved(partial));
    }

    // 5. Deadlock guard: a pass that made no progress and scheduled no new fetch cannot converge — the fetched batch
    //    was garbage / irrelevant to the outstanding wants.
    if to_fetch.is_empty() && !progress {
        return Err(OotleSdkError::Resolution(
            "input resolver made no progress — outstanding wants cannot be satisfied".to_string(),
        ));
    }

    // 6. Otherwise, ask the host for the next batch. Record what we're asking for so the next pass can detect omissions
    //    as definitive misses (point 2 above). `to_fetch` already holds the *concrete* ids we discovered we still need
    //    — crucially the vault ids read out of a component on this pass — so surface it to the host as `fetch_ids` (the
    //    want list alone only names a want's seed, e.g. the component, and a host fetching that forever can't
    //    converge).
    partial.pending_fetch = dedup_ids(to_fetch);
    let fetch_ids = partial.pending_fetch.iter().map(ToString::to_string).collect();
    let want_list = WantList(partial.wants.clone());
    Ok(Resolution::NeedMore {
        partial,
        want_list,
        fetch_ids,
    })
}

/// Dispatches one want to its resolver. Returns `true` when the want is satisfied (and any inputs it
/// contributes have been pushed to `resolved`); `false` when more fetches (`to_fetch`) are needed.
fn resolve_one(
    want: &WantItem,
    cache: &HashMap<SubstateId, Option<SubstateValue>>,
    resolved: &mut Vec<SubstateRequirement>,
    to_fetch: &mut Vec<SubstateId>,
) -> Result<bool, OotleSdkError> {
    match want {
        WantItem::SpecificSubstate { substate_id, required } => {
            resolve_specific(substate_id, *required, cache, resolved, to_fetch)
        },
        WantItem::VaultForResource {
            component_address,
            resource_address,
            required,
        } => resolve_vault(
            component_address,
            resource_address,
            *required,
            cache,
            resolved,
            to_fetch,
        ),
        WantItem::AllComponentVaults { component_address } => {
            resolve_all_vaults(component_address, cache, resolved, to_fetch)
        },
        // Dispatched earlier in apply_fetched_substates_with_secrets (needs spend secrets); exhaustive
        // arm kept so a new variant forces a compile error here.
        WantItem::StealthUtxo { .. } => Err(OotleSdkError::Resolution(
            "stealth UTXO wants must be resolved via apply_fetched_substates_with_secrets".to_string(),
        )),
    }
}

/// Resolves a [`WantItem::SpecificSubstate`].
fn resolve_specific(
    substate_id: &str,
    required: bool,
    cache: &HashMap<SubstateId, Option<SubstateValue>>,
    resolved: &mut Vec<SubstateRequirement>,
    to_fetch: &mut Vec<SubstateId>,
) -> Result<bool, OotleSdkError> {
    let id = SubstateId::from_str_checked(substate_id)?;

    // Required substates are added without a fetch — validators reject a bad input anyway (saves a
    // round-trip).
    if required {
        push_unique(resolved, SubstateRequirement::unversioned(id));
        return Ok(true);
    }

    match cache.get(&id) {
        Some(Some(_)) => {
            push_unique(resolved, SubstateRequirement::unversioned(id));
            Ok(true)
        },
        // Fetched-but-absent: optional, so satisfied without an input.
        Some(None) => Ok(true),
        // Never fetched: ask the host.
        None => {
            to_fetch.push(id);
            Ok(false)
        },
    }
}

/// Resolves a [`WantItem::VaultForResource`].
fn resolve_vault(
    component_address: &str,
    resource_address: &str,
    required: bool,
    cache: &HashMap<SubstateId, Option<SubstateValue>>,
    resolved: &mut Vec<SubstateRequirement>,
    to_fetch: &mut Vec<SubstateId>,
) -> Result<bool, OotleSdkError> {
    let component_id = SubstateId::from_str_checked(component_address)?;
    let resource = ResourceAddress::from_str_checked(resource_address)?;

    let component = match cache.get(&component_id) {
        None => {
            // Component not yet fetched.
            to_fetch.push(component_id);
            return Ok(false);
        },
        Some(None) => {
            // Component definitively absent.
            if required {
                return Err(OotleSdkError::Resolution(format!(
                    "required component substate '{component_address}' not found"
                )));
            }
            return Ok(true);
        },
        Some(Some(value)) => match value {
            SubstateValue::Component(c) => c,
            // The id resolved to a non-component substate — an inconsistent indexer view. Error
            // rather than silently satisfy.
            _ => {
                return Err(OotleSdkError::Resolution(format!(
                    "expected a component at '{component_address}' but the indexer returned a different substate kind"
                )));
            },
        },
    };

    let vault_ids = component_vault_ids(component)?;

    let mut have_all_vaults = true;
    let mut matched = false;
    for vault_id in vault_ids {
        match cache.get(&vault_id) {
            Some(Some(value)) => {
                if let SubstateValue::Vault(vault) = value &&
                    *vault.resource_address() == resource
                {
                    push_unique(resolved, SubstateRequirement::unversioned(vault_id.clone()));
                    if resource != TARI_TOKEN {
                        push_unique(resolved, SubstateRequirement::unversioned(resource));
                    }
                    matched = true;
                }
            },
            // Fetched-but-absent vault: keep looking at the others.
            Some(None) => {},
            None => {
                have_all_vaults = false;
                to_fetch.push(vault_id);
            },
        }
    }

    if matched {
        return Ok(true);
    }
    if !have_all_vaults {
        // Still need to fetch some vaults before we can decide.
        return Ok(false);
    }
    if required {
        return Err(OotleSdkError::Resolution(format!(
            "required vault for resource '{resource_address}' in component '{component_address}' does not exist"
        )));
    }
    // Optional and not found after checking every vault → satisfied (the create instruction handles it).
    Ok(true)
}

/// Resolves a [`WantItem::AllComponentVaults`].
fn resolve_all_vaults(
    component_address: &str,
    cache: &HashMap<SubstateId, Option<SubstateValue>>,
    resolved: &mut Vec<SubstateRequirement>,
    to_fetch: &mut Vec<SubstateId>,
) -> Result<bool, OotleSdkError> {
    let component_id = SubstateId::from_str_checked(component_address)?;

    let component = match cache.get(&component_id) {
        None => {
            to_fetch.push(component_id);
            return Ok(false);
        },
        Some(None) => {
            return Err(OotleSdkError::Resolution(format!(
                "component substate '{component_address}' not found"
            )));
        },
        Some(Some(value)) => match value {
            SubstateValue::Component(c) => c,
            _ => {
                return Err(OotleSdkError::Resolution(format!(
                    "expected a component at '{component_address}' but the indexer returned a different substate kind"
                )));
            },
        },
    };

    // The called component is itself an input (parity with the synchronous builder's
    // `add_input_for_component_ref`). Without this, a method call on a component the host does not own
    // is rejected at execution with SUBSTATE_NOT_FOUND.
    push_unique(resolved, SubstateRequirement::unversioned(component_id));

    let vault_ids = component_vault_ids(component)?;

    let mut all_cached = true;
    for vault_id in vault_ids {
        match cache.get(&vault_id) {
            Some(Some(value)) => {
                if let SubstateValue::Vault(vault) = value {
                    push_unique(resolved, SubstateRequirement::unversioned(vault_id.clone()));
                    let resource = *vault.resource_address();
                    if resource != TARI_TOKEN {
                        push_unique(resolved, SubstateRequirement::unversioned(resource));
                    }
                }
            },
            Some(None) => {
                // The component referenced this vault but the indexer reports it missing — an
                // inconsistent view, a hard error.
                return Err(OotleSdkError::Resolution(format!(
                    "vault '{vault_id}' referenced by component '{component_address}' was not found"
                )));
            },
            None => {
                all_cached = false;
                to_fetch.push(vault_id);
            },
        }
    }

    Ok(all_cached)
}

/// Reads a component's state for the vault ids it references.
fn component_vault_ids(component: &tari_engine_types::component::Component) -> Result<Vec<SubstateId>, OotleSdkError> {
    let indexed = component
        .body
        .to_indexed_well_known_types()
        .map_err(|e| OotleSdkError::Parse(format!("failed to index component state: {e}")))?;
    Ok(indexed.vault_ids().iter().map(|v| SubstateId::Vault(*v)).collect())
}

/// Pushes an input only if not already present (stable `Vec`, `IndexSet`-like de-dup).
fn push_unique(resolved: &mut Vec<SubstateRequirement>, req: SubstateRequirement) {
    if !resolved.contains(&req) {
        resolved.push(req);
    }
}

/// De-dups a list of substate ids, preserving first-seen order.
fn dedup_ids(ids: Vec<SubstateId>) -> Vec<SubstateId> {
    let mut out: Vec<SubstateId> = Vec::with_capacity(ids.len());
    for id in ids {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// Local parse seams so this module never panics on a malformed boundary string.
trait FromStrChecked: Sized {
    fn from_str_checked(s: &str) -> Result<Self, OotleSdkError>;
}

impl FromStrChecked for SubstateId {
    fn from_str_checked(s: &str) -> Result<Self, OotleSdkError> {
        use std::str::FromStr;
        SubstateId::from_str(s).map_err(|e| OotleSdkError::Parse(format!("invalid substate id '{s}': {e}")))
    }
}

impl FromStrChecked for ResourceAddress {
    fn from_str_checked(s: &str) -> Result<Self, OotleSdkError> {
        use std::str::FromStr;
        ResourceAddress::from_str(s).map_err(|e| OotleSdkError::Parse(format!("invalid resource address '{s}': {e}")))
    }
}

#[cfg(test)]
mod tests {
    use tari_crypto::{keys::PublicKey as _, ristretto::RistrettoPublicKey, tari_utilities::ByteArray};
    use tari_engine_types::{
        component::{Component, ComponentBody, ComponentHeader},
        resource_container::ResourceContainer,
        substate::SubstateValue,
        vault::Vault,
    };
    use tari_template_lib_types::{
        Amount,
        EntityId,
        ObjectKey,
        ResourceAddress,
        SubstateOwnerRule,
        VaultId,
        access_rules::ComponentAccessRules,
        crypto::RistrettoPublicKeyBytes,
    };

    use super::*;
    use crate::{
        builder::build_public_transfer_unsigned,
        types::{
            address::{ComponentAddressStr, ResourceAddressStr},
            bytes::PublicKeyBytes,
            intent::{InputRef, TransferRecipient},
            numeric::BoundaryAmount,
        },
    };

    // --- Fixtures ---------------------------------------------------------------------------------

    fn from_component() -> ComponentAddress {
        ComponentAddress::new(ObjectKey::from_array([0xaa; ObjectKey::LENGTH]))
    }

    fn resource() -> ResourceAddress {
        ResourceAddress::new(ObjectKey::from_array([0xbb; ObjectKey::LENGTH]))
    }

    fn from_vault_id() -> VaultId {
        VaultId::new(ObjectKey::from_array([0xcc; ObjectKey::LENGTH]))
    }

    /// A canonical recipient public key (low scalar) + the account component it derives to.
    fn recipient_pk() -> RistrettoPublicKeyBytes {
        let mut b = [0u8; 32];
        b[0] = 7;
        let sk = tari_crypto::ristretto::RistrettoSecretKey::from_canonical_bytes(&b).unwrap();
        let pk = RistrettoPublicKey::from_secret_key(&sk);
        RistrettoPublicKeyBytes::from_bytes(pk.as_bytes()).unwrap()
    }

    fn recipient_pk_bytes() -> PublicKeyBytes {
        PublicKeyBytes::from_bytes(recipient_pk().as_bytes()).unwrap()
    }

    fn recipient_component() -> ComponentAddress {
        derive_component_address_from_public_key(&ACCOUNT_TEMPLATE_ADDRESS, &recipient_pk())
    }

    fn intent_resolved() -> PublicTransferIntent {
        PublicTransferIntent {
            from_account: ComponentAddressStr::from_internal(&from_component()),
            recipient: TransferRecipient::PublicKey(recipient_pk_bytes()),
            resource_address: ResourceAddressStr::from_internal(&resource()),
            amount: BoundaryAmount::new(1_000_000),
            fee: BoundaryAmount::new(2000),
            inputs: vec![],
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
        }
    }

    /// A `Component` whose state references `vault_ids`, JSON-encoded the way the indexer would
    /// hand it back.
    fn component_substate_json(vault_ids: &[VaultId]) -> serde_json::Value {
        let state = tari_bor::to_value(&vault_ids.to_vec()).unwrap();
        let component = Component {
            header: ComponentHeader {
                template_address: ACCOUNT_TEMPLATE_ADDRESS,
                owner_rule: SubstateOwnerRule::None,
                access_rules: ComponentAccessRules::new(),
                entity_id: EntityId::from_array([0u8; EntityId::LENGTH]),
            },
            body: ComponentBody::from_cbor_value(state),
        };
        serde_json::to_value(SubstateValue::Component(component)).unwrap()
    }

    fn vault_substate_json(resource: ResourceAddress) -> serde_json::Value {
        let vault = Vault::new(ResourceContainer::public_fungible(resource, Amount::new(1_000_000)));
        serde_json::to_value(SubstateValue::Vault(vault)).unwrap()
    }

    fn fetched(id: impl ToString, value: serde_json::Value) -> FetchedSubstate {
        FetchedSubstate {
            substate_id: id.to_string(),
            version: 0,
            substate_value: value,
        }
    }

    // --- (a) want derivation ----------------------------------------------------------------------

    #[test]
    fn derives_the_four_item_want_set() {
        let (_partial, wants) =
            build_public_transfer_unsigned_with_wants(Network::Esmeralda, &intent_resolved()).unwrap();
        assert_eq!(wants.0.len(), 4, "exactly the 4-item public-transfer want set");

        // From-component: required specific substate (the fee + withdraw call it).
        assert_eq!(wants.0[0], WantItem::SpecificSubstate {
            substate_id: from_component().to_string(),
            required: true,
        });
        // From-vault: required, on the from-component, for the resource.
        assert_eq!(wants.0[1], WantItem::VaultForResource {
            component_address: from_component().to_string(),
            resource_address: resource().to_string(),
            required: true,
        });
        // Recipient component: optional specific substate.
        assert_eq!(wants.0[2], WantItem::SpecificSubstate {
            substate_id: recipient_component().to_string(),
            required: false,
        });
        // Recipient vault: optional.
        assert_eq!(wants.0[3], WantItem::VaultForResource {
            component_address: recipient_component().to_string(),
            resource_address: resource().to_string(),
            required: false,
        });
    }

    fn unwrap_need_more(res: Resolution) -> (PartialTransaction, WantList) {
        match res {
            Resolution::NeedMore { partial, want_list, .. } => (partial, want_list),
            Resolution::Resolved(_) => panic!("expected NeedMore, got Resolved"),
        }
    }

    fn unwrap_resolved(res: Resolution) -> PartialTransaction {
        match res {
            Resolution::Resolved(p) => p,
            Resolution::NeedMore { want_list, .. } => panic!("expected Resolved, got NeedMore({want_list:?})"),
        }
    }

    // --- (b) happy resolve ------------------------------------------------------------------------

    #[test]
    fn happy_resolve_folds_inputs_and_resolves() {
        let (partial, wants) =
            build_public_transfer_unsigned_with_wants(Network::Esmeralda, &intent_resolved()).unwrap();
        assert_eq!(wants.0.len(), 4);

        // Round 1: the host fetched the seeds (from-component + recipient-component). The
        // from-component carries one vault; the recipient component is absent (omitted from the
        // batch). After this pass: recipient-component want satisfied-as-absent (optional);
        // recipient-vault want satisfied-as-absent (its component is absent); from-vault want still
        // needs the vault fetched.
        let round1 = vec![fetched(
            SubstateId::Component(from_component()),
            component_substate_json(&[from_vault_id()]),
        )];
        let (partial, next_wants) = unwrap_need_more(apply_fetched_substates(partial, &round1).unwrap());
        assert_eq!(next_wants.0.len(), 1, "only the from-vault want remains");
        assert_eq!(next_wants.0[0], WantItem::VaultForResource {
            component_address: from_component().to_string(),
            resource_address: resource().to_string(),
            required: true,
        });

        // Round 2: the host fetches the from-vault. It holds the resource → the want resolves.
        let round2 = vec![fetched(
            SubstateId::Vault(from_vault_id()),
            vault_substate_json(resource()),
        )];
        let partial = unwrap_resolved(apply_fetched_substates(partial, &round2).unwrap());

        // The resolved inputs: the from-component, the from-vault, plus the resource (it is not TARI_TOKEN).
        let resolved = partial.resolved_inputs();
        assert!(
            resolved
                .iter()
                .any(|r| *r.substate_id() == SubstateId::Component(from_component()))
        );
        assert!(
            resolved
                .iter()
                .any(|r| *r.substate_id() == SubstateId::Vault(from_vault_id()))
        );
        assert!(
            resolved
                .iter()
                .any(|r| *r.substate_id() == SubstateId::Resource(resource()))
        );

        // Folded into the unsigned tx.
        let unsigned = partial.into_unsigned();
        assert!(
            unsigned
                .inputs()
                .iter()
                .any(|i| *i.substate_id() == SubstateId::Vault(from_vault_id()))
        );
        assert!(
            unsigned
                .inputs()
                .iter()
                .any(|i| *i.substate_id() == SubstateId::Resource(resource()))
        );
    }

    // --- (c) need-more ----------------------------------------------------------------------------

    #[test]
    fn reveals_component_but_not_its_vault_needs_more() {
        let (partial, _wants) =
            build_public_transfer_unsigned_with_wants(Network::Esmeralda, &intent_resolved()).unwrap();

        // Only the from-component (with a vault id) is returned; its vault is not yet fetched.
        let round1 = vec![fetched(
            SubstateId::Component(from_component()),
            component_substate_json(&[from_vault_id()]),
        )];
        let res = apply_fetched_substates(partial, &round1).unwrap();
        let (_partial, fetch_ids, next) = match res {
            Resolution::NeedMore {
                partial,
                fetch_ids,
                want_list,
            } => (partial, fetch_ids, want_list),
            Resolution::Resolved(_) => panic!("expected NeedMore, got Resolved"),
        };
        // The from-vault want is still outstanding (its vault must still be fetched).
        assert!(
            next.0
                .iter()
                .any(|w| matches!(w, WantItem::VaultForResource { required: true, .. }))
        );
        // `fetch_ids` exposes the concrete vault id the resolver discovered inside the component, so a
        // host can fetch it and converge. The want list's seed alone would only name the component
        // again.
        assert_eq!(
            fetch_ids,
            vec![SubstateId::Vault(from_vault_id()).to_string()],
            "NeedMore must surface the discovered vault id, not just re-seed the component"
        );
        assert!(
            !next
                .seed_substate_ids()
                .contains(&SubstateId::Vault(from_vault_id()).to_string()),
            "the want-list seed deliberately does NOT name the vault — that is exactly why fetch_ids exists"
        );
    }

    // --- (d) deadlock / garbage -------------------------------------------------------------------

    #[test]
    fn garbage_fetch_that_satisfies_nothing_is_a_resolution_error() {
        let (partial, _wants) =
            build_public_transfer_unsigned_with_wants(Network::Esmeralda, &intent_resolved()).unwrap();

        // The from-component is reported absent → the required from-vault want can never resolve.
        // (We feed an empty batch; the pending from-component seed is marked absent.)
        let err = apply_fetched_substates(partial, &[]).unwrap_err();
        assert_eq!(err.code(), "RESOLUTION");
    }

    #[test]
    fn malformed_substate_value_is_a_parse_error() {
        let (partial, _wants) =
            build_public_transfer_unsigned_with_wants(Network::Esmeralda, &intent_resolved()).unwrap();
        let bad = vec![fetched(
            SubstateId::Component(from_component()),
            serde_json::json!({ "NotARealVariant": 1 }),
        )];
        let err = apply_fetched_substates(partial, &bad).unwrap_err();
        assert_eq!(err.code(), "PARSE");
    }

    #[test]
    fn malformed_substate_id_in_fetch_is_a_parse_error() {
        let (partial, _wants) =
            build_public_transfer_unsigned_with_wants(Network::Esmeralda, &intent_resolved()).unwrap();
        let bad = vec![fetched("not-a-substate-id", component_substate_json(&[]))];
        let err = apply_fetched_substates(partial, &bad).unwrap_err();
        assert_eq!(err.code(), "PARSE");
    }

    // --- (e) parity: resolved vs explicit ---------------------------------------------------------

    #[test]
    fn resolved_unsigned_matches_explicit_for_equivalent_input_set() {
        // The resolved path yields these inputs for our fixture: the from-component, the from-vault,
        // and the resource. An explicit intent with exactly those inputs must produce a
        // byte-identical unsigned tx.
        let resolved_intent = intent_resolved();

        // Drive the resolved path to completion.
        let (partial, _w) = build_public_transfer_unsigned_with_wants(Network::Esmeralda, &resolved_intent).unwrap();
        let r1 = vec![fetched(
            SubstateId::Component(from_component()),
            component_substate_json(&[from_vault_id()]),
        )];
        let (partial, _w) = unwrap_need_more(apply_fetched_substates(partial, &r1).unwrap());
        let r2 = vec![fetched(
            SubstateId::Vault(from_vault_id()),
            vault_substate_json(resource()),
        )];
        let partial = unwrap_resolved(apply_fetched_substates(partial, &r2).unwrap());
        let resolved_unsigned = partial.into_unsigned();

        // Build the same transfer explicitly with the equivalent input set (same order the resolver
        // accumulates: from-component, then vault, then resource).
        let mut explicit_intent = intent_resolved();
        explicit_intent.inputs = vec![
            InputRef::unversioned(SubstateId::Component(from_component()).to_string()),
            InputRef::unversioned(SubstateId::Vault(from_vault_id()).to_string()),
            InputRef::unversioned(SubstateId::Resource(resource()).to_string()),
        ];
        let explicit_unsigned = build_public_transfer_unsigned(Network::Esmeralda, &explicit_intent).unwrap();

        // The unsigned forms (no signatures yet) must encode identically.
        let ra = tari_bor::encode(&resolved_unsigned).unwrap();
        let rb = tari_bor::encode(&explicit_unsigned).unwrap();
        assert_eq!(ra, rb, "resolved and explicit unsigned tx must be byte-identical");
    }

    // --- explicit short-circuit -------------------------------------------------------------------

    #[test]
    fn explicit_inputs_short_circuit_to_resolved_with_empty_wants() {
        let mut intent = intent_resolved();
        intent.inputs = vec![InputRef::versioned(from_component().to_string(), 0)];
        let (partial, wants) = build_public_transfer_unsigned_with_wants(Network::Esmeralda, &intent).unwrap();
        assert!(wants.0.is_empty(), "explicit path emits no wants");

        // An empty fetch batch resolves immediately.
        let partial = unwrap_resolved(apply_fetched_substates(partial, &[]).unwrap());
        let unsigned = partial.into_unsigned();
        assert!(unsigned.inputs().iter().any(|i| i.version() == Some(0)));
    }

    #[test]
    fn account_recipient_is_rejected() {
        let mut intent = intent_resolved();
        intent.recipient = TransferRecipient::Account(ComponentAddressStr::from_internal(&from_component()));
        let err = build_public_transfer_unsigned_with_wants(Network::Esmeralda, &intent).unwrap_err();
        assert_eq!(err.code(), "INVALID");
    }

    // --- (k) stealth integration: the public-transfer path is unchanged with empty secrets --------

    #[test]
    fn public_transfer_path_unchanged_with_empty_secrets() {
        // The full resolved walk via `apply_fetched_substates_with_secrets(.., &empty)` must match
        // the public `apply_fetched_substates`.
        let (partial, _w) = build_public_transfer_unsigned_with_wants(Network::Esmeralda, &intent_resolved()).unwrap();
        let r1 = vec![fetched(
            SubstateId::Component(from_component()),
            component_substate_json(&[from_vault_id()]),
        )];
        let secrets = HashMap::new();
        let (partial, _w) = match apply_fetched_substates_with_secrets(partial, &r1, &secrets).unwrap() {
            Resolution::NeedMore { partial, want_list, .. } => (partial, want_list),
            Resolution::Resolved(_) => panic!("expected NeedMore"),
        };
        let r2 = vec![fetched(
            SubstateId::Vault(from_vault_id()),
            vault_substate_json(resource()),
        )];
        let partial = unwrap_resolved(apply_fetched_substates_with_secrets(partial, &r2, &secrets).unwrap());
        assert!(
            partial
                .resolved_inputs()
                .iter()
                .any(|r| *r.substate_id() == SubstateId::Vault(from_vault_id()))
        );
        // No stealth state accumulated on the public path.
        assert!(partial.stealth_input_witnesses().is_empty());
        assert!(partial.seal_signer().is_none());
    }
}
