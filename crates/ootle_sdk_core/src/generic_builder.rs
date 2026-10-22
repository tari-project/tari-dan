//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Generic instruction front-end: lowers a [`GenericTransactionIntent`] into a
//! [`PartialTransaction`] + [`WantList`] for the two-phase resolve/seal pipeline.
//!
//! Each [`InstructionSpec`] is lowered to an [`Instruction`](tari_ootle_transaction::Instruction);
//! workspace labels are resolved to numeric ids at the call point (the builder owns the label→id map,
//! assigning ids in instruction order). A label is registered when an instruction produces a
//! workspace slot ([`InstructionSpec::PutLastInstructionOutputOnWorkspace`]) and read back via
//! [`TransactionBuilder::get_workspace_offset_id_from_named_arg`] when a later [`ArgValue::Workspace`]
//! consumes it.
//!
//! ## Two phases
//!
//! Fee instructions run first (the fee payment appended last) and their workspace is dropped before
//! the main instructions run, so each phase tracks its own bound workspace labels. A self-funding
//! transaction (create + fund + pay from the new account) lives entirely in the fee phase.
//!
//! ## Want derivation
//!
//! A [`FeeSource::FromAccount`] needs that account's component + TARI vault; a workspace/bucket fee
//! source needs none (the payer is produced in-transaction). Each address-targeted
//! [`InstructionSpec::CallMethod`] needs the called component's vaults. An explicit input set
//! short-circuits to an empty want list.

use std::collections::HashSet;

use tari_ootle_common_types::Epoch;
use tari_ootle_transaction::{
    ComponentReference,
    Instruction,
    TransactionBuilder,
    args::InstructionArg,
    builder::named_args::parse_workspace_key,
};
use tari_template_lib_types::{OwnerRule, TemplateAddress, crypto::RistrettoPublicKeyBytes};

use crate::{
    PartialTransaction,
    WantItem,
    WantList,
    types::{
        error::OotleSdkError,
        generic_intent::{
            ArgValue,
            ComponentRef,
            FeeSource,
            GenericTransactionIntent,
            InstructionSpec,
            OwnerRuleSpec,
            encode_arg,
            workspace_arg,
        },
        network::Network,
    },
};

/// Round 1 for a generic intent: emit the instruction list onto a builder and derive the want set,
/// returning a [`PartialTransaction`] + [`WantList`] the host drives through
/// `apply_fetched_substates → … → seal_and_encode_public_transfer[_with_seed]`.
///
/// **Explicit-input short-circuit:** if `intent.inputs` is non-empty this returns a partial already
/// carrying those inputs and an empty want list (the first `apply_fetched_substates` with an empty
/// batch yields `Resolved`).
///
/// Errors map to [`OotleSdkError`]; nothing panics on bad input.
pub fn build_unsigned_instructions_with_wants(
    network: Network,
    intent: &GenericTransactionIntent,
) -> Result<(PartialTransaction, WantList), OotleSdkError> {
    build_unsigned_instructions_with_extra_wants(network, intent, Vec::new())
}

/// As [`build_unsigned_instructions_with_wants`], but appends `extra_wants` to the derived want set.
///
/// Appends `extra_wants` the caller knows about but `derive_wants` cannot infer from the generic
/// instruction list (e.g. an optional recipient account/vault). `extra_wants` is ignored on the
/// explicit-input short-circuit (that path resolves nothing). `intent.extra_inputs` is always merged
/// in regardless of path.
pub(crate) fn build_unsigned_instructions_with_extra_wants(
    network: Network,
    intent: &GenericTransactionIntent,
    extra_wants: Vec<WantItem>,
) -> Result<(PartialTransaction, WantList), OotleSdkError> {
    // Cheap structural check (blob indices in range) before any lowering.
    intent.validate_blob_indices()?;

    let builder = lower_intent(network, intent)?;
    let unsigned = builder.build_unsigned();
    let extra_inputs = extra_inputs_to_internal(intent)?;

    // Explicit-input short-circuit: caller supplied an input set, so skip resolution. `extra_inputs`
    // is still merged in (it is always additive), de-duped by id so the first-seen entry wins —
    // matching the resolution path's `push_unique`.
    if !intent.inputs.is_empty() {
        let mut inputs = inputs_to_internal(intent)?;
        for req in extra_inputs {
            if !inputs.contains(&req) {
                inputs.push(req);
            }
        }
        let partial = PartialTransaction::new_with_explicit_inputs(unsigned, inputs);
        return Ok((partial, WantList(Vec::new())));
    }

    let mut wants = derive_wants(intent)?;
    wants.0.extend(extra_wants);
    let wants = WantList(dedup_wants(wants.0));
    let partial = PartialTransaction::new_for_wants_with_extra_inputs(unsigned, wants.clone(), extra_inputs);
    Ok((partial, wants))
}

/// Builds a [`TransactionBuilder`] for `network`: lowers the fee phase (fee instructions + the fee
/// payment) and the main phase, and threads the epoch window + dry-run flag.
fn lower_intent(network: Network, intent: &GenericTransactionIntent) -> Result<TransactionBuilder, OotleSdkError> {
    // The fee phase runs first and its workspace is dropped before the main phase, so each phase
    // tracks its own bound workspace labels (matching the engine's per-phase workspace).
    let fee_instructions = lower_fee_phase(network, intent)?;
    let mut builder =
        TransactionBuilder::new(network.as_byte(), Epoch(intent.max_epoch)).with_fee_instructions(fee_instructions);

    // Track the main-phase workspace labels bound so far (by a `PutLastInstructionOutputOnWorkspace`)
    // so a later `ArgValue::Workspace(label)` is validated before we ask the builder for its id — the
    // builder's `get_workspace_offset_id_from_named_arg` panics on an unbound label, so we never hand
    // it one.
    let mut bound_labels: HashSet<String> = HashSet::new();
    for instr in &intent.instructions {
        builder = lower_instruction(builder, intent, instr, &mut bound_labels)?;
    }

    Ok(builder
        .with_min_epoch(intent.min_epoch.map(Epoch))
        .with_dry_run(intent.dry_run))
}

/// Lowers the fee phase to a flat instruction list: every `fee_instructions` spec followed by the
/// fee payment. The fee payment is appended last so any account it pays from has been set up by the
/// preceding fee instructions.
fn lower_fee_phase(network: Network, intent: &GenericTransactionIntent) -> Result<Vec<Instruction>, OotleSdkError> {
    let mut builder = TransactionBuilder::new(network.as_byte(), Epoch(intent.max_epoch));
    let mut bound_labels: HashSet<String> = HashSet::new();
    for instr in &intent.fee_instructions {
        builder = lower_instruction(builder, intent, instr, &mut bound_labels)?;
    }
    builder = lower_fee_payment(builder, intent, &bound_labels)?;
    // `into_instructions` returns this builder's main-phase list — the flat fee-phase sequence.
    Ok(builder.into_instructions())
}

/// Appends the fee-payment instruction to the fee phase per the intent's [`FeeSource`].
///
/// `FromAccount`/`FromWorkspaceComponent` lower to a `CallMethod(.., "pay_fee", [fee])` (an address
/// reference, or a workspace reference resolved like any [`ComponentRef::Workspace`]); `FromBucket`
/// lowers to `PayFeeFromBucket`. A workspace/bucket label must be bound by a prior fee-phase
/// `PutLastInstructionOutputOnWorkspace`.
fn lower_fee_payment(
    builder: TransactionBuilder,
    intent: &GenericTransactionIntent,
    bound_labels: &HashSet<String>,
) -> Result<TransactionBuilder, OotleSdkError> {
    let call = match &intent.fee_payment {
        FeeSource::FromAccount(account) => ComponentReference::from(account.to_internal()?),
        FeeSource::FromWorkspaceComponent { label } => {
            resolve_component_ref(&builder, &ComponentRef::Workspace(label.clone()), bound_labels)?
        },
        FeeSource::FromBucket { label } => {
            ensure_label_bound(label, bound_labels)?;
            let bucket = builder.get_workspace_offset_id_from_named_arg(label.clone());
            return Ok(builder.add_instruction(Instruction::PayFeeFromBucket { bucket }));
        },
    };
    Ok(builder.add_instruction(Instruction::CallMethod {
        call,
        method: parse_function_name("pay_fee")?,
        args: vec![encode_arg(&ArgValue::Amount(intent.fee.as_u64()))?],
    }))
}

/// Validates a workspace label is bound by a prior `PutLastInstructionOutputOnWorkspace` in the same
/// phase, so the builder's panicking `get_workspace_offset_id_from_named_arg` is only ever handed a
/// registered label.
fn ensure_label_bound(label: &str, bound_labels: &HashSet<String>) -> Result<(), OotleSdkError> {
    let parsed = parse_workspace_key(label.to_string())
        .map_err(|e| OotleSdkError::Validation(format!("invalid workspace label '{label}': {e}")))?;
    if !bound_labels.contains(&parsed.name) {
        return Err(OotleSdkError::Validation(format!(
            "workspace label '{}' is not bound by a prior PutLastInstructionOutputOnWorkspace instruction",
            parsed.name
        )));
    }
    Ok(())
}

/// Lowers one [`InstructionSpec`] onto the builder, resolving workspace labels at the call point.
fn lower_instruction(
    builder: TransactionBuilder,
    intent: &GenericTransactionIntent,
    instr: &InstructionSpec,
    bound_labels: &mut HashSet<String>,
) -> Result<TransactionBuilder, OotleSdkError> {
    match instr {
        InstructionSpec::CallFunction {
            template_address,
            function,
            args,
        } => {
            let address = parse_template_address(template_address)?;
            let function = parse_function_name(function)?;
            let args = lower_args(&builder, args, bound_labels)?;
            Ok(builder.add_instruction(Instruction::CallFunction {
                address,
                function,
                args,
            }))
        },
        InstructionSpec::CallMethod { call, method, args } => {
            let method = parse_function_name(method)?;
            let args = lower_args(&builder, args, bound_labels)?;
            let call = resolve_component_ref(&builder, call, bound_labels)?;
            Ok(builder.add_instruction(Instruction::CallMethod { call, method, args }))
        },
        InstructionSpec::CreateAccount {
            owner_public_key,
            owner_rule,
            bucket_workspace_id,
        } => {
            let owner_public_key: RistrettoPublicKeyBytes = owner_public_key.to_internal();
            // Plain create-or-fetch unless an owner rule or bucket is requested, in which case the
            // custom form carries them (a bucket label deposits its workspace bucket on creation).
            if owner_rule.is_none() && bucket_workspace_id.is_none() {
                return Ok(builder.create_account(owner_public_key));
            }
            let owner_rule = owner_rule.as_ref().map(lower_owner_rule);
            let bucket = match bucket_workspace_id {
                Some(label) => {
                    ensure_label_bound(label, bound_labels)?;
                    Some(label.clone())
                },
                None => None,
            };
            Ok(builder.create_account_custom(owner_public_key, owner_rule, None, bucket))
        },
        InstructionSpec::PublishTemplate {
            blob_index,
            metadata_hash,
        } => {
            // `validate_blob_indices` (called up front) guarantees the index is in range.
            let blob = &intent.blobs[*blob_index as usize];
            // `publish_template` auto-registers the binary as an unnamed blob and emits the
            // `PublishTemplate` instruction referencing it. We register one blob per `PublishTemplate`
            // (matching the intent's blob list one-to-one).
            match metadata_hash {
                Some(hex) => Ok(builder.publish_template_with_metadata(blob.bytes.clone(), parse_metadata_hash(hex)?)),
                None => Ok(builder.publish_template(blob.bytes.clone())),
            }
        },
        InstructionSpec::PutLastInstructionOutputOnWorkspace { key } => {
            // Let the builder assign + register the numeric workspace id for this label (so a later
            // `ArgValue::Workspace(key)` resolves to the same id at its call point). Record the label
            // as bound so a later `Workspace(key)` validates before we ask the builder. Parse the key
            // through the same `parse_workspace_key` the consumer uses, so we register the bare name
            // (`"foo"` from a `"foo.0"` key) — producer and consumer agree on the name regardless of
            // any `.offset` suffix.
            let parsed = parse_workspace_key(key.clone())
                .map_err(|e| OotleSdkError::Validation(format!("invalid workspace label '{key}': {e}")))?;
            // Register + store under the bare name so a later consumer's `parsed.name` lookup matches
            // (the builder stores the key verbatim; a dotted producer key would otherwise never resolve).
            bound_labels.insert(parsed.name.clone());
            Ok(builder.put_last_instruction_output_on_workspace(parsed.name))
        },
    }
}

/// Lowers a typed arg list to engine `InstructionArg`s, resolving workspace labels at the call point.
///
/// Literals are encoded via [`encode_arg`]. [`ArgValue::Workspace`] labels are resolved against the
/// builder's label→id map (registered by the `PutLastInstructionOutputOnWorkspace` that produced the
/// slot) and carried via [`workspace_arg`].
fn lower_args(
    builder: &TransactionBuilder,
    args: &[ArgValue],
    bound_labels: &HashSet<String>,
) -> Result<Vec<InstructionArg>, OotleSdkError> {
    args.iter()
        .map(|arg| match arg {
            ArgValue::Workspace(label) => resolve_workspace_arg(builder, label, bound_labels),
            literal => encode_arg(literal),
        })
        .collect()
}

/// Resolves an [`ArgValue::Workspace`] label to its numeric carrier at the call point.
///
/// Honors the builder's `name.offset` key convention. A label that names no workspace slot bound by a
/// prior `PutLastInstructionOutputOnWorkspace` ⇒ [`OotleSdkError::Validation`]. We validate against
/// `bound_labels` *first* so the builder's `get_workspace_offset_id_from_named_arg` (which panics on
/// an unbound label) is only ever handed a label it has registered.
fn resolve_workspace_arg(
    builder: &TransactionBuilder,
    label: &str,
    bound_labels: &HashSet<String>,
) -> Result<InstructionArg, OotleSdkError> {
    ensure_label_bound(label, bound_labels)?;
    // Safe: validated above. The builder resolves the label (honoring any `.offset`) to the
    // `WorkspaceOffsetId` it registered for the producing instruction; `workspace_arg` builds the
    // carrier — the single `ArgValue::Workspace → InstructionArg::Workspace` mapping.
    let offset_id = builder.get_workspace_offset_id_from_named_arg(label.to_string());
    Ok(workspace_arg(offset_id))
}

/// Resolves a boundary [`ComponentRef`] to the engine [`ComponentReference`] at the call point.
///
/// `Address` parses the component address; `Workspace` validates the label is bound in this phase
/// then resolves it to the numeric id the builder registered. A component reference carries no offset,
/// so a `name.offset` label is a [`OotleSdkError::Validation`].
fn resolve_component_ref(
    builder: &TransactionBuilder,
    call: &ComponentRef,
    bound_labels: &HashSet<String>,
) -> Result<ComponentReference, OotleSdkError> {
    match call {
        ComponentRef::Address(addr) => Ok(addr.to_internal()?.into()),
        ComponentRef::Workspace(label) => {
            ensure_label_bound(label, bound_labels)?;
            let offset_id = builder.get_workspace_offset_id_from_named_arg(label.clone());
            if offset_id.offset().is_some() {
                return Err(OotleSdkError::Validation(format!(
                    "workspace component label '{label}' must not carry a '.offset' suffix"
                )));
            }
            Ok(ComponentReference::Workspace(offset_id.id()))
        },
    }
}

/// Lowers a boundary [`OwnerRuleSpec`] to the engine [`OwnerRule`].
fn lower_owner_rule(rule: &OwnerRuleSpec) -> OwnerRule {
    match rule {
        OwnerRuleSpec::OwnedBySigner => OwnerRule::OwnedBySigner,
        OwnerRuleSpec::None => OwnerRule::None,
        OwnerRuleSpec::ByPublicKey(pk) => OwnerRule::ByPublicKey(pk.to_internal()),
    }
}

/// Parses a lowercase-hex metadata multihash into the engine `MetadataHash`.
fn parse_metadata_hash(hex: &str) -> Result<tari_ootle_template_metadata::MetadataHash, OotleSdkError> {
    let bytes = hex::decode(hex).map_err(|e| OotleSdkError::Parse(format!("invalid metadata_hash hex: {e}")))?;
    tari_ootle_template_metadata::MetadataHash::from_bytes(&bytes)
        .ok_or_else(|| OotleSdkError::Parse("metadata_hash is not a valid multihash".to_string()))
}

/// Parses a template address into the engine [`TemplateAddress`] (a 32-byte hash). Accepts both the
/// canonical `template_<hex>` form (as emitted in diffs/substate ids) and bare hex.
fn parse_template_address(s: &str) -> Result<TemplateAddress, OotleSdkError> {
    let hex = s
        .strip_prefix(tari_template_lib_types::address_prefixes::TEMPLATE)
        .and_then(|rest| rest.strip_prefix('_'))
        .unwrap_or(s);
    TemplateAddress::from_hex(hex).map_err(|_| OotleSdkError::Parse(format!("invalid template address '{s}'")))
}

/// Parses a function/method name into the bounded engine [`FunctionName`](tari_template_lib_types::FunctionName).
fn parse_function_name(s: &str) -> Result<tari_template_lib_types::FunctionName, OotleSdkError> {
    use std::convert::TryFrom;
    tari_template_lib_types::FunctionName::try_from(s.to_string())
        .map_err(|_| OotleSdkError::Validation(format!("function/method name '{s}' exceeds the length limit")))
}

/// Converts the intent's explicit input set to internal `InputDeclaration`s.
fn inputs_to_internal(
    intent: &GenericTransactionIntent,
) -> Result<Vec<tari_ootle_common_types::InputDeclaration>, OotleSdkError> {
    intent
        .inputs
        .iter()
        .map(crate::types::intent::InputRef::to_internal)
        .collect()
}

/// Converts the intent's additive `extra_inputs` to internal `InputDeclaration`s.
fn extra_inputs_to_internal(
    intent: &GenericTransactionIntent,
) -> Result<Vec<tari_ootle_common_types::InputDeclaration>, OotleSdkError> {
    intent
        .extra_inputs
        .iter()
        .map(crate::types::intent::InputRef::to_internal)
        .collect()
}

/// Derives the want set for a generic intent.
///
/// - `FeeSource::FromAccount` needs that account's TARI vault — required. `FromWorkspaceComponent` / `FromBucket`
///   derive no fee want: the paying component is not on-ledger yet (it is produced in the fee phase), so the fee is
///   self-funded and there is nothing to fetch.
/// - Each address-targeted [`InstructionSpec::CallMethod`] (in either phase) additionally needs the called component's
///   vaults (`AllComponentVaults`), so a `withdraw`/`deposit`/`pay_fee` against it can resolve. Workspace-targeted
///   calls reference a component produced in-transaction — there is nothing on-ledger to fetch, so no want is derived
///   for them.
///
/// Wants are emitted in a deterministic order (fee first, then per-instruction, fee phase before
/// main) and de-duplicated by their canonical string form so a host fetches each substate once.
fn derive_wants(intent: &GenericTransactionIntent) -> Result<WantList, OotleSdkError> {
    let mut wants: Vec<WantItem> = Vec::new();

    // The fee account: its component (the `pay_fee` call targets it, so it must be a declared input)
    // plus its TARI vault to charge from.
    if let FeeSource::FromAccount(account) = &intent.fee_payment {
        let account = account.to_internal()?;
        wants.push(WantItem::SpecificSubstate {
            substate_id: account.to_string(),
            required: true,
        });
        wants.push(WantItem::VaultForResource {
            component_address: account.to_string(),
            resource_address: tari_template_lib_types::constants::TARI_TOKEN.to_string(),
            required: true,
        });
    }

    // Every on-ledger component a method is called on needs its vaults discovered so the call can
    // resolve; every substate address appearing in a literal call arg needs pinning too. The fee
    // phase is walked before the main phase to keep the order deterministic.
    for instr in intent.fee_instructions.iter().chain(&intent.instructions) {
        match instr {
            InstructionSpec::CallMethod {
                call: ComponentRef::Address(component_address),
                args,
                ..
            } => {
                let component =
                    crate::types::address::ComponentAddressStr::parse(component_address.as_str())?.to_internal()?;
                wants.push(WantItem::AllComponentVaults {
                    component_address: component.to_string(),
                });
                push_arg_substate_wants(args, &mut wants);
            },
            InstructionSpec::CallMethod { args, .. } | InstructionSpec::CallFunction { args, .. } => {
                push_arg_substate_wants(args, &mut wants);
            },
            _ => {},
        }
    }

    Ok(WantList(dedup_wants(wants)))
}

/// Emits a `SpecificSubstate` want (required) for every substate address referenced by a literal call
/// arg, so a call passing a component/resource address as a literal arg gets that substate pinned.
/// `Workspace` args carry no literal bytes and are skipped; a literal that does not decode to
/// referenced substates contributes nothing (best-effort decoding).
fn push_arg_substate_wants(args: &[ArgValue], wants: &mut Vec<WantItem>) {
    for arg in args {
        let Ok(encoded) = encode_arg(arg) else { continue };
        let Some(bytes) = encoded.as_literal_bytes() else {
            continue;
        };
        let Ok(indexed) = tari_engine_types::indexed_value::IndexedValue::from_raw(bytes) else {
            continue;
        };
        for id in indexed.referenced_substates().filter(|id| id.is_declarable_input()) {
            wants.push(WantItem::SpecificSubstate {
                substate_id: id.to_string(),
                required: true,
            });
        }
    }
}

/// De-dups wants by their canonical string form, preserving first-seen order. A `CallMethod` on the
/// fee account would otherwise duplicate the fee's `VaultForResource` component fetch; the resolver
/// already de-dups resolved inputs, but trimming here keeps the want list (and its seed-fetch set)
/// minimal.
fn dedup_wants(wants: Vec<WantItem>) -> Vec<WantItem> {
    let mut out: Vec<WantItem> = Vec::with_capacity(wants.len());
    for want in wants {
        if !out.contains(&want) {
            out.push(want);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use tari_crypto::{keys::PublicKey as _, ristretto::RistrettoPublicKey, tari_utilities::ByteArray};
    use tari_engine_types::{
        SubstateVersion,
        component::{Component, ComponentBody, ComponentHeader},
        resource_container::ResourceContainer,
        substate::{SubstateId, SubstateValue},
        vault::Vault,
    };
    use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
    use tari_template_lib_types::{
        Amount,
        ComponentAddress,
        EntityId,
        ObjectKey,
        ResourceAddress,
        SubstateOwnerRule,
        VaultId,
        access_rules::ComponentAccessRules,
        constants::TARI_TOKEN,
    };

    use super::*;
    use crate::{
        FetchedSubstate,
        Resolution,
        apply_fetched_substates,
        resolved_transfer::resolved_unsigned,
        types::{
            address::{ComponentAddressStr, ResourceAddressStr},
            bytes::PublicKeyBytes,
            intent::InputRef,
            numeric::BoundaryAmount,
        },
    };

    // --- Fixed, canonical test material ----------------------------------------------------------

    fn fixed_scalar(seed: u8) -> tari_crypto::ristretto::RistrettoSecretKey {
        let mut b = [0u8; 32];
        b[0] = seed;
        tari_crypto::ristretto::RistrettoSecretKey::from_canonical_bytes(&b).expect("low scalar is canonical")
    }

    fn from_component() -> ComponentAddress {
        ComponentAddress::new(ObjectKey::from_array([0xaa; ObjectKey::LENGTH]))
    }

    fn resource() -> ResourceAddress {
        ResourceAddress::new(ObjectKey::from_array([0xbb; ObjectKey::LENGTH]))
    }

    fn from_vault_id() -> VaultId {
        VaultId::new(ObjectKey::from_array([0xcc; ObjectKey::LENGTH]))
    }

    fn recipient_pk_bytes() -> PublicKeyBytes {
        let pk = RistrettoPublicKey::from_secret_key(&fixed_scalar(7));
        PublicKeyBytes::from_bytes(pk.as_bytes()).unwrap()
    }

    /// A `FromAccount` fee paid from the given component.
    fn fee_from(component: ComponentAddress) -> FeeSource {
        FeeSource::FromAccount(ComponentAddressStr::from_internal(&component))
    }

    /// A plain create-account spec (no owner rule / bucket).
    fn create_account(pk: PublicKeyBytes) -> InstructionSpec {
        InstructionSpec::CreateAccount {
            owner_public_key: pk,
            owner_rule: None,
            bucket_workspace_id: None,
        }
    }

    /// A `CallMethod` targeting a component address.
    fn call_addr(component: ComponentAddress, method: &str, args: Vec<ArgValue>) -> InstructionSpec {
        InstructionSpec::CallMethod {
            call: ComponentRef::Address(ComponentAddressStr::from_internal(&component)),
            method: method.to_string(),
            args,
        }
    }

    /// A withdraw → put-on-workspace → create-account intent (the expressible transfer shape).
    fn transfer_intent(inputs: Vec<InputRef>) -> GenericTransactionIntent {
        GenericTransactionIntent {
            fee: BoundaryAmount::new(2000),
            fee_payment: fee_from(from_component()),
            fee_instructions: vec![],
            instructions: vec![
                call_addr(from_component(), "withdraw", vec![
                    ArgValue::Address(ResourceAddressStr::from_internal(&resource()).0),
                    ArgValue::Amount(1_000_000),
                ]),
                InstructionSpec::PutLastInstructionOutputOnWorkspace {
                    key: "bucket".to_string(),
                },
                create_account(recipient_pk_bytes()),
            ],
            blobs: vec![],
            inputs,
            extra_inputs: vec![],
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
        }
    }

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

    fn vault_substate_json(res: ResourceAddress) -> serde_json::Value {
        let vault = Vault::new(ResourceContainer::public_fungible(res, Amount::new(1_000_000)));
        serde_json::to_value(SubstateValue::Vault(vault)).unwrap()
    }

    fn fetched(id: impl ToString, value: serde_json::Value) -> FetchedSubstate {
        FetchedSubstate {
            substate_id: id.to_string(),
            version: 0,
            substate_value: value,
        }
    }

    // --- Want derivation -------------------------------------------------------------------------

    #[test]
    fn derives_fee_vault_plus_method_component_vaults() {
        let (_partial, wants) =
            build_unsigned_instructions_with_wants(Network::Esmeralda, &transfer_intent(vec![])).unwrap();
        // Fee account component (required) + its fee vault (required) + the from-component's vaults
        // (the withdraw target) + the resource referenced by the withdraw's literal address arg. The
        // fee account IS the from-component, so the `SpecificSubstate`, `VaultForResource`, and
        // `AllComponentVaults` are distinct wants.
        assert_eq!(
            wants.0.len(),
            4,
            "fee account component + fee vault + the method component's vaults + the arg resource"
        );
        assert_eq!(wants.0[0], WantItem::SpecificSubstate {
            substate_id: from_component().to_string(),
            required: true,
        });
        assert_eq!(wants.0[1], WantItem::VaultForResource {
            component_address: from_component().to_string(),
            resource_address: TARI_TOKEN.to_string(),
            required: true,
        });
        assert_eq!(wants.0[2], WantItem::AllComponentVaults {
            component_address: from_component().to_string(),
        });
        // The `Address(resource)` literal arg to `withdraw` is pinned (parity with auto-fill).
        assert_eq!(wants.0[3], WantItem::SpecificSubstate {
            substate_id: resource().to_string(),
            required: true,
        });
    }

    #[test]
    fn explicit_inputs_short_circuit_to_empty_wants() {
        let intent = transfer_intent(vec![InputRef::versioned(from_component().to_string(), 0)]);
        let (partial, wants) = build_unsigned_instructions_with_wants(Network::Esmeralda, &intent).unwrap();
        assert!(wants.0.is_empty(), "explicit path emits no wants");
        // An empty fetch batch resolves immediately.
        let resolved = match apply_fetched_substates(partial, &[]).unwrap() {
            Resolution::Resolved(p) => p,
            Resolution::NeedMore { .. } => panic!("expected Resolved"),
        };
        let unsigned = resolved.into_unsigned();
        assert!(
            unsigned
                .inputs()
                .iter()
                .any(|i| i.version() == Some(SubstateVersion::ZERO))
        );
    }

    #[test]
    fn parse_template_address_accepts_canonical_and_bare_hex() {
        let hex = "77".repeat(32);
        let canonical = format!("template_{hex}");
        let from_canonical = parse_template_address(&canonical).unwrap();
        let from_bare = parse_template_address(&hex).unwrap();
        assert_eq!(from_canonical, from_bare);
        assert_eq!(parse_template_address("nope").unwrap_err().code(), "PARSE");
    }

    // --- Instruction lowering --------------------------------------------------------------------

    #[test]
    fn lowers_to_fee_plus_three_instructions() {
        let (partial, _w) = build_unsigned_instructions_with_wants(
            Network::Esmeralda,
            &transfer_intent(vec![InputRef::versioned(from_component().to_string(), 0)]),
        )
        .unwrap();
        let resolved = match apply_fetched_substates(partial, &[]).unwrap() {
            Resolution::Resolved(p) => p,
            Resolution::NeedMore { .. } => panic!("expected Resolved"),
        };
        let unsigned = resolved.into_unsigned();
        assert_eq!(unsigned.fee_instructions().len(), 1, "one pay_fee fee instruction");
        assert_eq!(
            unsigned.instructions().len(),
            3,
            "withdraw + put-on-workspace + create-account"
        );
    }

    /// A workspace pipe: `PutLastInstructionOutputOnWorkspace("b")` then a `CallMethod` consuming
    /// `Workspace("b")` must encode the workspace ref to the id the producer registered.
    #[test]
    fn workspace_pipe_resolves_at_call_point() {
        let intent = GenericTransactionIntent {
            fee: BoundaryAmount::new(2000),
            fee_payment: fee_from(from_component()),
            fee_instructions: vec![],
            instructions: vec![
                call_addr(from_component(), "withdraw", vec![
                    ArgValue::Address(ResourceAddressStr::from_internal(&resource()).0),
                    ArgValue::Amount(1_000_000),
                ]),
                InstructionSpec::PutLastInstructionOutputOnWorkspace { key: "b".to_string() },
                call_addr(from_component(), "deposit", vec![ArgValue::Workspace("b".to_string())]),
            ],
            blobs: vec![],
            inputs: vec![InputRef::versioned(from_component().to_string(), 0)],
            extra_inputs: vec![],
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
        };
        let (partial, _w) = build_unsigned_instructions_with_wants(Network::Esmeralda, &intent).unwrap();
        let unsigned = match apply_fetched_substates(partial, &[]).unwrap() {
            Resolution::Resolved(p) => p.into_unsigned(),
            Resolution::NeedMore { .. } => panic!("expected Resolved"),
        };
        // The deposit instruction's single arg must be the workspace carrier for id 0 (the first and
        // only registered label).
        let deposit = &unsigned.instructions()[2];
        match deposit {
            Instruction::CallMethod { args, .. } => {
                assert_eq!(args.len(), 1);
                assert_eq!(args[0], InstructionArg::workspace(0, None), "Workspace(b) → id 0");
            },
            other => panic!("expected CallMethod, got {other:?}"),
        }
    }

    /// A consumer that references a bound label with a `.offset` suffix (`"b.0"`) must resolve
    /// against the bare registered name (`"b"`), not be falsely rejected.
    #[test]
    fn dotted_offset_workspace_ref_resolves_against_bare_name() {
        let intent = GenericTransactionIntent {
            fee: BoundaryAmount::new(2000),
            fee_payment: fee_from(from_component()),
            fee_instructions: vec![],
            instructions: vec![
                call_addr(from_component(), "withdraw", vec![
                    ArgValue::Address(ResourceAddressStr::from_internal(&resource()).0),
                    ArgValue::Amount(1_000_000),
                ]),
                InstructionSpec::PutLastInstructionOutputOnWorkspace { key: "b".to_string() },
                // `.0` selects offset 0 of the workspace item bound under "b".
                call_addr(from_component(), "deposit", vec![ArgValue::Workspace(
                    "b.0".to_string(),
                )]),
            ],
            blobs: vec![],
            inputs: vec![InputRef::versioned(from_component().to_string(), 0)],
            extra_inputs: vec![],
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
        };
        let (partial, _w) = build_unsigned_instructions_with_wants(Network::Esmeralda, &intent).unwrap();
        let unsigned = match apply_fetched_substates(partial, &[]).unwrap() {
            Resolution::Resolved(p) => p.into_unsigned(),
            Resolution::NeedMore { .. } => panic!("expected Resolved"),
        };
        match &unsigned.instructions()[2] {
            Instruction::CallMethod { args, .. } => {
                assert_eq!(args.len(), 1);
                assert_eq!(
                    args[0],
                    InstructionArg::workspace(0, Some(0)),
                    "Workspace(b.0) → id 0, offset 0"
                );
            },
            other => panic!("expected CallMethod, got {other:?}"),
        }
    }

    #[test]
    fn unbound_workspace_label_is_a_validation_error() {
        let intent = GenericTransactionIntent {
            fee: BoundaryAmount::new(2000),
            fee_payment: fee_from(from_component()),
            fee_instructions: vec![],
            instructions: vec![call_addr(from_component(), "deposit", vec![ArgValue::Workspace(
                "never_bound".to_string(),
            )])],
            blobs: vec![],
            inputs: vec![InputRef::versioned(from_component().to_string(), 0)],
            extra_inputs: vec![],
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
        };
        let err = build_unsigned_instructions_with_wants(Network::Esmeralda, &intent).unwrap_err();
        assert_eq!(err.code(), "VALIDATION");
    }

    #[test]
    fn out_of_range_blob_index_is_a_validation_error() {
        let intent = GenericTransactionIntent {
            fee: BoundaryAmount::new(2000),
            fee_payment: fee_from(from_component()),
            fee_instructions: vec![],
            instructions: vec![InstructionSpec::PublishTemplate {
                blob_index: 0,
                metadata_hash: None,
            }],
            blobs: vec![],
            inputs: vec![InputRef::versioned(from_component().to_string(), 0)],
            extra_inputs: vec![],
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
        };
        let err = build_unsigned_instructions_with_wants(Network::Esmeralda, &intent).unwrap_err();
        assert_eq!(err.code(), "VALIDATION");
    }

    #[test]
    fn bad_component_address_is_a_parse_error() {
        let mut intent = transfer_intent(vec![InputRef::versioned(from_component().to_string(), 0)]);
        intent.fee_payment = FeeSource::FromAccount(ComponentAddressStr("not-a-component".to_string()));
        let err = build_unsigned_instructions_with_wants(Network::Esmeralda, &intent).unwrap_err();
        assert_eq!(err.code(), "PARSE");
    }

    // --- Superset proof: the generic front-end equals the builder's native lowering --------------

    /// A generic intent lowered through `build_unsigned_instructions_with_wants` lowers to the
    /// identical canonical unsigned transaction as the same instruction sequence hand-built on the
    /// builder's own methods — the generic front-end produces the same transaction as the hand-built
    /// flow (the signatures, added at seal time, are random and so compared elsewhere).
    #[test]
    fn generic_lowers_identically_to_hand_built_builder() {
        let intent = transfer_intent(vec![InputRef::versioned(from_component().to_string(), 0)]);

        // Generic path: lower → resolve (empty batch, explicit inputs) → canonical unsigned.
        let (partial, _w) = build_unsigned_instructions_with_wants(Network::Esmeralda, &intent).unwrap();
        let generic = match apply_fetched_substates(partial, &[]).unwrap() {
            Resolution::Resolved(p) => resolved_unsigned(p).unwrap(),
            Resolution::NeedMore { .. } => panic!("expected Resolved"),
        };

        // Hand-built path: the IDENTICAL instruction sequence via the builder's own methods.
        let hand = {
            use tari_ootle_transaction::args;
            // Mirror the generic path exactly: inputs are carried ONLY by `new_with_explicit_inputs`
            // (the generic `lower_intent` never calls `with_inputs`), so the builder emits just the
            // instructions and the explicit inputs are folded in at seal time.
            let unsigned = TransactionBuilder::new(Network::Esmeralda.as_byte(), Epoch(1))
                .pay_fee_from_component(from_component(), Amount::new(2000))
                .call_method(from_component(), "withdraw", args![resource(), Amount::new(1_000_000)])
                .put_last_instruction_output_on_workspace("bucket")
                .create_account(recipient_pk_bytes().to_internal())
                .build_unsigned();
            let partial = PartialTransaction::new_with_explicit_inputs(unsigned, vec![
                InputRef::versioned(from_component().to_string(), 0)
                    .to_internal()
                    .unwrap(),
            ]);
            match apply_fetched_substates(partial, &[]).unwrap() {
                Resolution::Resolved(p) => resolved_unsigned(p).unwrap(),
                Resolution::NeedMore { .. } => panic!("expected Resolved"),
            }
        };

        assert_eq!(
            serde_json::to_value(&generic).unwrap(),
            serde_json::to_value(&hand).unwrap(),
            "generic front-end must lower to the identical canonical unsigned tx as the hand-built builder"
        );
    }

    /// Multi-round resolution stays canonical (the `canonicalize_inputs` sort holds): a resolved
    /// (want-list) intent resolved in two rounds yields the identical unsigned tx as the single-round
    /// resolution.
    #[test]
    fn multi_round_resolution_stays_canonical() {
        let intent = transfer_intent(vec![]);

        // Single-round: fee-vault component + its vault in one batch.
        let single = {
            let (partial, _w) = build_unsigned_instructions_with_wants(Network::Esmeralda, &intent).unwrap();
            let batch = vec![
                fetched(
                    SubstateId::Component(from_component()),
                    component_substate_json(&[from_vault_id()]),
                ),
                fetched(SubstateId::Vault(from_vault_id()), vault_substate_json(TARI_TOKEN)),
            ];
            match apply_fetched_substates(partial, &batch).unwrap() {
                Resolution::Resolved(p) => resolved_unsigned(p).unwrap(),
                Resolution::NeedMore { .. } => panic!("expected Resolved"),
            }
        };

        // Two-round: component first (reveals the vault id), then the discovered vault.
        let two_round = {
            let (partial, _w) = build_unsigned_instructions_with_wants(Network::Esmeralda, &intent).unwrap();
            let r1 = vec![fetched(
                SubstateId::Component(from_component()),
                component_substate_json(&[from_vault_id()]),
            )];
            let partial = match apply_fetched_substates(partial, &r1).unwrap() {
                Resolution::NeedMore { partial, .. } => partial,
                Resolution::Resolved(_) => panic!("expected NeedMore"),
            };
            let r2 = vec![fetched(
                SubstateId::Vault(from_vault_id()),
                vault_substate_json(TARI_TOKEN),
            )];
            match apply_fetched_substates(partial, &r2).unwrap() {
                Resolution::Resolved(p) => resolved_unsigned(p).unwrap(),
                Resolution::NeedMore { .. } => panic!("expected Resolved"),
            }
        };

        assert_eq!(
            serde_json::to_value(&single).unwrap(),
            serde_json::to_value(&two_round).unwrap(),
            "single-round and multi-round resolution must produce the identical canonical unsigned tx"
        );
    }

    // --- Self-funding fee + two-phase lowering ---------------------------------------------------

    fn faucet_component() -> ComponentAddress {
        ComponentAddress::new(ObjectKey::from_array([0xfa; ObjectKey::LENGTH]))
    }

    /// The self-funding faucet shape: the fee phase creates an account, puts it on the workspace, the
    /// faucet funds it, and the fee is charged from that workspace component.
    fn self_funding_intent(fee_payment: FeeSource) -> GenericTransactionIntent {
        GenericTransactionIntent {
            fee: BoundaryAmount::new(2000),
            fee_payment,
            fee_instructions: vec![
                create_account(recipient_pk_bytes()),
                InstructionSpec::PutLastInstructionOutputOnWorkspace {
                    key: "acct".to_string(),
                },
                call_addr(faucet_component(), "take", vec![ArgValue::Workspace(
                    "acct".to_string(),
                )]),
            ],
            instructions: vec![],
            blobs: vec![],
            inputs: vec![InputRef::versioned(faucet_component().to_string(), 0)],
            extra_inputs: vec![],
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
        }
    }

    #[test]
    fn workspace_fee_derives_no_fee_vault_want() {
        let intent = self_funding_intent(FeeSource::FromWorkspaceComponent {
            label: "acct".to_string(),
        });
        let wants = derive_wants(&intent).unwrap();
        // No VaultForResource (the paying account is not on-ledger); only the faucet's vaults, which
        // is targeted by address.
        assert!(
            !wants.0.iter().any(|w| matches!(w, WantItem::VaultForResource { .. })),
            "a workspace-funded fee derives no fee vault want"
        );
        assert_eq!(wants.0, vec![WantItem::AllComponentVaults {
            component_address: faucet_component().to_string(),
        }]);
    }

    #[test]
    fn workspace_fee_payment_targets_workspace_component_in_fee_phase() {
        let intent = self_funding_intent(FeeSource::FromWorkspaceComponent {
            label: "acct".to_string(),
        });
        let (partial, _w) = build_unsigned_instructions_with_wants(Network::Esmeralda, &intent).unwrap();
        let unsigned = match apply_fetched_substates(partial, &[]).unwrap() {
            Resolution::Resolved(p) => p.into_unsigned(),
            Resolution::NeedMore { .. } => panic!("expected Resolved"),
        };
        // The whole self-funding sequence is in the fee phase; the main phase is empty.
        assert!(unsigned.instructions().is_empty(), "no main-phase instructions");
        let fee = unsigned.fee_instructions();
        assert_eq!(fee.len(), 4, "create + put + take + pay_fee");
        // The fee payment (last) is a pay_fee on the workspace component (id 0, the only bound label).
        match &fee[3] {
            Instruction::CallMethod { call, method, .. } => {
                assert_eq!(method.to_string(), "pay_fee");
                assert_eq!(*call, ComponentReference::Workspace(0));
            },
            other => panic!("expected CallMethod pay_fee, got {other:?}"),
        }
    }

    #[test]
    fn from_bucket_fee_lowers_to_pay_fee_from_bucket() {
        let intent = self_funding_intent(FeeSource::FromBucket {
            label: "acct".to_string(),
        });
        let (partial, _w) = build_unsigned_instructions_with_wants(Network::Esmeralda, &intent).unwrap();
        let unsigned = match apply_fetched_substates(partial, &[]).unwrap() {
            Resolution::Resolved(p) => p.into_unsigned(),
            Resolution::NeedMore { .. } => panic!("expected Resolved"),
        };
        let fee = unsigned.fee_instructions();
        assert!(
            matches!(fee.last(), Some(Instruction::PayFeeFromBucket { .. })),
            "FromBucket lowers to a trailing PayFeeFromBucket"
        );
    }

    #[test]
    fn unbound_workspace_fee_label_is_a_validation_error() {
        let intent = self_funding_intent(FeeSource::FromWorkspaceComponent {
            label: "never_bound".to_string(),
        });
        let err = build_unsigned_instructions_with_wants(Network::Esmeralda, &intent).unwrap_err();
        assert_eq!(err.code(), "VALIDATION");
    }

    /// A component reference has no offset slot, so a `.offset`-suffixed fee payer label is rejected.
    #[test]
    fn offset_suffixed_workspace_fee_label_is_a_validation_error() {
        let intent = self_funding_intent(FeeSource::FromWorkspaceComponent {
            label: "acct.0".to_string(),
        });
        let err = build_unsigned_instructions_with_wants(Network::Esmeralda, &intent).unwrap_err();
        assert_eq!(err.code(), "VALIDATION");
    }

    #[test]
    fn offset_suffixed_workspace_call_target_is_a_validation_error() {
        let intent = GenericTransactionIntent {
            fee: BoundaryAmount::new(2000),
            fee_payment: fee_from(from_component()),
            fee_instructions: vec![],
            instructions: vec![
                create_account(recipient_pk_bytes()),
                InstructionSpec::PutLastInstructionOutputOnWorkspace { key: "new".to_string() },
                InstructionSpec::CallMethod {
                    call: ComponentRef::Workspace("new.0".to_string()),
                    method: "deposit".to_string(),
                    args: vec![],
                },
            ],
            blobs: vec![],
            inputs: vec![InputRef::versioned(from_component().to_string(), 0)],
            extra_inputs: vec![],
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
        };
        let err = build_unsigned_instructions_with_wants(Network::Esmeralda, &intent).unwrap_err();
        assert_eq!(err.code(), "VALIDATION");
    }

    #[test]
    fn workspace_call_method_target_is_not_wanted() {
        // A `CallMethod` on a workspace component references an in-tx component — no want is derived,
        // and it lowers to a workspace-targeted call.
        let intent = GenericTransactionIntent {
            fee: BoundaryAmount::new(2000),
            fee_payment: fee_from(from_component()),
            fee_instructions: vec![],
            instructions: vec![
                create_account(recipient_pk_bytes()),
                InstructionSpec::PutLastInstructionOutputOnWorkspace { key: "new".to_string() },
                InstructionSpec::CallMethod {
                    call: ComponentRef::Workspace("new".to_string()),
                    method: "deposit".to_string(),
                    args: vec![],
                },
            ],
            blobs: vec![],
            inputs: vec![InputRef::versioned(from_component().to_string(), 0)],
            extra_inputs: vec![],
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
        };
        let wants = derive_wants(&intent).unwrap();
        // Only the fee account's component + vault — the workspace-targeted call contributes no want.
        assert_eq!(wants.0, vec![
            WantItem::SpecificSubstate {
                substate_id: from_component().to_string(),
                required: true,
            },
            WantItem::VaultForResource {
                component_address: from_component().to_string(),
                resource_address: TARI_TOKEN.to_string(),
                required: true,
            }
        ]);
        let (partial, _w) = build_unsigned_instructions_with_wants(Network::Esmeralda, &intent).unwrap();
        let unsigned = match apply_fetched_substates(partial, &[]).unwrap() {
            Resolution::Resolved(p) => p.into_unsigned(),
            Resolution::NeedMore { .. } => panic!("expected Resolved"),
        };
        match &unsigned.instructions()[2] {
            Instruction::CallMethod { call, .. } => assert_eq!(*call, ComponentReference::Workspace(0)),
            other => panic!("expected CallMethod, got {other:?}"),
        }
    }

    #[test]
    fn create_account_with_owner_rule_and_bucket_lowers_custom() {
        // CreateAccount with an owner rule + a bucket workspace label lowers to the custom form and
        // deposits the bucket atomically.
        let intent = GenericTransactionIntent {
            fee: BoundaryAmount::new(2000),
            fee_payment: fee_from(from_component()),
            fee_instructions: vec![],
            instructions: vec![
                call_addr(from_component(), "withdraw", vec![
                    ArgValue::Address(ResourceAddressStr::from_internal(&resource()).0),
                    ArgValue::Amount(1_000_000),
                ]),
                InstructionSpec::PutLastInstructionOutputOnWorkspace { key: "b".to_string() },
                InstructionSpec::CreateAccount {
                    owner_public_key: recipient_pk_bytes(),
                    owner_rule: Some(OwnerRuleSpec::None),
                    bucket_workspace_id: Some("b".to_string()),
                },
            ],
            blobs: vec![],
            inputs: vec![InputRef::versioned(from_component().to_string(), 0)],
            extra_inputs: vec![],
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
        };
        let (partial, _w) = build_unsigned_instructions_with_wants(Network::Esmeralda, &intent).unwrap();
        let unsigned = match apply_fetched_substates(partial, &[]).unwrap() {
            Resolution::Resolved(p) => p.into_unsigned(),
            Resolution::NeedMore { .. } => panic!("expected Resolved"),
        };
        match &unsigned.instructions()[2] {
            Instruction::CreateAccount {
                owner_rule,
                bucket_workspace_id,
                ..
            } => {
                assert_eq!(*owner_rule, Some(OwnerRule::None));
                assert!(bucket_workspace_id.is_some(), "the bucket workspace id is carried");
            },
            other => panic!("expected CreateAccount, got {other:?}"),
        }
    }

    #[test]
    fn create_account_with_bucket_unbound_label_is_a_validation_error() {
        let intent = GenericTransactionIntent {
            fee: BoundaryAmount::new(2000),
            fee_payment: fee_from(from_component()),
            fee_instructions: vec![],
            instructions: vec![InstructionSpec::CreateAccount {
                owner_public_key: recipient_pk_bytes(),
                owner_rule: None,
                bucket_workspace_id: Some("missing".to_string()),
            }],
            blobs: vec![],
            inputs: vec![InputRef::versioned(from_component().to_string(), 0)],
            extra_inputs: vec![],
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
        };
        let err = build_unsigned_instructions_with_wants(Network::Esmeralda, &intent).unwrap_err();
        assert_eq!(err.code(), "VALIDATION");
    }

    // --- Parity with the synchronous auto-fill builder -------------------------------------------

    fn foreign_component() -> ComponentAddress {
        ComponentAddress::new(ObjectKey::from_array([0xdd; ObjectKey::LENGTH]))
    }

    fn arg_resource() -> ResourceAddress {
        ResourceAddress::new(ObjectKey::from_array([0xee; ObjectKey::LENGTH]))
    }

    /// Every input the synchronous `with_auto_fill_inputs()` builder adds for a foreign `CallMethod` +
    /// a `CallFunction` with a literal address arg (the called component and the literal-arg substates)
    /// must also appear in the two-phase resolved set. The two-phase path may add *more* (the discovered
    /// component vaults, which auto-fill leaves to a manual `add_input`), so this asserts containment.
    #[test]
    fn two_phase_resolution_covers_auto_fill_inputs() {
        use std::collections::HashSet;

        use tari_ootle_transaction::args;

        // Auto-fill side: the two main instructions on the synchronous builder (no fee instruction, so
        // its inputs are exactly the component + the two literal-arg resources).
        let auto_inputs: HashSet<String> = TransactionBuilder::new(Network::Esmeralda.as_byte(), Epoch(1))
            .with_auto_fill_inputs()
            .call_method(foreign_component(), "do", args![arg_resource()])
            .call_function(ACCOUNT_TEMPLATE_ADDRESS, "make", args![resource()])
            .build_unsigned()
            .inputs()
            .iter()
            .map(|r| r.substate_id().to_string())
            .collect();

        // Two-phase side: the same two instructions through the generic intent, resolved.
        let intent = GenericTransactionIntent {
            fee: BoundaryAmount::new(2000),
            fee_payment: fee_from(from_component()),
            fee_instructions: vec![],
            instructions: vec![
                call_addr(foreign_component(), "do", vec![ArgValue::Address(
                    ResourceAddressStr::from_internal(&arg_resource()).0,
                )]),
                InstructionSpec::CallFunction {
                    template_address: ACCOUNT_TEMPLATE_ADDRESS.to_string(),
                    function: "make".to_string(),
                    args: vec![ArgValue::Address(ResourceAddressStr::from_internal(&resource()).0)],
                },
            ],
            blobs: vec![],
            inputs: vec![],
            extra_inputs: vec![],
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
        };
        let (partial, _w) = build_unsigned_instructions_with_wants(Network::Esmeralda, &intent).unwrap();

        // Round 1: the fee account (reveals its vault id) + the foreign component (no vaults).
        let r1 = vec![
            fetched(
                SubstateId::Component(from_component()),
                component_substate_json(&[from_vault_id()]),
            ),
            fetched(SubstateId::Component(foreign_component()), component_substate_json(&[])),
        ];
        let partial = match apply_fetched_substates(partial, &r1).unwrap() {
            Resolution::NeedMore { partial, .. } => partial,
            Resolution::Resolved(_) => panic!("expected NeedMore (fee vault still to fetch)"),
        };
        // Round 2: the discovered fee vault.
        let r2 = vec![fetched(
            SubstateId::Vault(from_vault_id()),
            vault_substate_json(TARI_TOKEN),
        )];
        let resolved = match apply_fetched_substates(partial, &r2).unwrap() {
            Resolution::Resolved(p) => p,
            Resolution::NeedMore { .. } => panic!("expected Resolved"),
        };
        let two_phase: HashSet<String> = resolved
            .resolved_inputs()
            .iter()
            .map(|r| r.substate_id().to_string())
            .collect();

        assert!(
            auto_inputs.is_subset(&two_phase),
            "two-phase resolution must cover every auto-fill input.\n  auto-fill: {auto_inputs:?}\n  two-phase: \
             {two_phase:?}"
        );
        // The called foreign component and both literal-arg resources.
        assert!(
            two_phase.contains(&foreign_component().to_string()),
            "foreign component must be resolved"
        );
        assert!(
            two_phase.contains(&arg_resource().to_string()),
            "method literal-arg resource must be resolved"
        );
        assert!(
            two_phase.contains(&resource().to_string()),
            "function literal-arg resource must be resolved"
        );
    }
}
