//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Seal/encode a **resolved** [`PartialTransaction`].
//!
//! Input resolution produces a [`PartialTransaction`]
//! ([`build_public_transfer_unsigned_with_wants`] → [`apply_fetched_substates`]). This module takes a
//! fully-resolved partial all the way to submit-ready BOR bytes + transaction id by reusing the
//! seal/encode leaves ([`seal_public_transfer`] + [`bor_encode`]), so the resolved path produces the
//! same instruction/input bytes as the explicit path (the signatures are random).
//!
//! ## Canonical input sort
//!
//! The resolver accumulates inputs into a stable `Vec` in walk order, but the seal path owns the
//! canonical sort: before the bytes are signed/hashed, this module sorts the resolved inputs into a
//! deterministic order (by their canonical [`SubstateRequirement`] string form). Without this, two
//! resolutions that fold inputs in a different order would seal input-divergent transactions. The sort
//! is applied only on the seal/encode path so the resolver's `into_unsigned` accumulation order stays
//! untouched.

use tari_ootle_transaction::UnsignedTransaction;

use crate::{
    PartialTransaction,
    keys::PublicTransferKeys,
    public_transfer::EncodedPublicTransfer,
    tx::{bor_encode, seal_public_transfer},
    types::{
        bytes::{EncodedTransactionBytes, TransactionIdBytes},
        error::OotleSdkError,
    },
};

/// Seals a resolved partial with the account key (random nonce) and BOR-encodes it. The bytes/id are
/// **not** reproducible — the safe default for real submission.
///
/// The partial must be fully resolved (no outstanding **required** wants). A partial still carrying
/// wants ⇒ [`OotleSdkError::Resolution`] (rather than silently sealing an incomplete input set).
pub fn seal_and_encode_public_transfer(
    partial: PartialTransaction,
    keys: &PublicTransferKeys,
) -> Result<EncodedPublicTransfer, OotleSdkError> {
    let unsigned = resolved_unsigned(partial)?;
    let (transaction, id) = seal_public_transfer(unsigned, keys)?;
    encode(transaction, id)
}

/// Folds a resolved partial into its [`UnsignedTransaction`] and applies the canonical input sort.
/// Errors if the partial is **not** fully resolved.
///
/// `pub(crate)` so the co-sign path derives the identical canonical unsigned transaction the seal
/// path seals.
pub(crate) fn resolved_unsigned(partial: PartialTransaction) -> Result<UnsignedTransaction, OotleSdkError> {
    if !partial.outstanding_wants().is_empty() {
        return Err(OotleSdkError::Resolution(format!(
            "cannot seal an unresolved transaction: {} want(s) still outstanding — call apply_fetched_substates until \
             it returns Resolved",
            partial.outstanding_wants().len(),
        )));
    }
    let mut unsigned = partial.into_unsigned();
    canonicalize_inputs(&mut unsigned);
    Ok(unsigned)
}

/// Canonically sorts the unsigned tx's inputs into a deterministic order, by the canonical
/// [`tari_ootle_common_types::SubstateRequirement`] string form (`<substate_id>:<version|?>`), which
/// is stable, total, and language-neutral.
///
/// This is the **only** place the resolved-path input order is fixed; the seal that follows hashes the
/// sorted set, so two resolutions of the same logical input set produce byte-identical instruction +
/// input bytes.
pub(crate) fn canonicalize_inputs(unsigned: &mut UnsignedTransaction) {
    let UnsignedTransaction::V1(v1) = unsigned;
    let mut inputs: Vec<_> = std::mem::take(&mut v1.inputs).into_iter().collect();
    inputs.sort_by_key(|a| a.to_string());
    // The set we took out already had unique substate ids: `IndexSet` de-dups on `SubstateRequirement`'s
    // `Hash`/`Eq`, which ignore the version field, and `into_unsigned` only folds an input in when no entry with
    // the same substate id is already present. So re-collecting the sorted `Vec` into the `IndexSet` below cannot
    // silently drop a same-id/different-version entry. Assert that invariant so a future caller that breaks it
    // (e.g. mixing a versioned explicit input with an unversioned resolved input for the same id) fails loudly in
    // debug rather than silently keeping whichever version sorts first.
    debug_assert!(
        inputs.windows(2).all(|w| w[0].substate_id() != w[1].substate_id()),
        "canonicalize_inputs: duplicate substate_id after sort — version-silent IndexSet de-dup would occur"
    );
    v1.inputs = inputs.into_iter().collect();
}

/// BOR-encodes the sealed transaction into the [`EncodedPublicTransfer`] output.
fn encode(
    transaction: tari_ootle_transaction::Transaction,
    id: tari_ootle_transaction::TransactionId,
) -> Result<EncodedPublicTransfer, OotleSdkError> {
    let bytes = bor_encode(transaction)?;
    Ok(EncodedPublicTransfer {
        encoded_transaction: EncodedTransactionBytes::from_vec(bytes),
        transaction_id: TransactionIdBytes::from_bytes(id.as_bytes()),
    })
}

#[cfg(test)]
mod tests {
    use tari_crypto::{
        keys::PublicKey as _,
        ristretto::{RistrettoPublicKey, RistrettoSecretKey},
        tari_utilities::ByteArray,
    };
    use tari_engine_types::{
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
        crypto::RistrettoPublicKeyBytes,
    };

    use super::*;
    use crate::{
        FetchedSubstate,
        apply_fetched_substates,
        inputs::{Resolution, build_public_transfer_unsigned_with_wants},
        tx::{seal_public_transfer, verify_all_signatures},
        types::{
            address::{ComponentAddressStr, ResourceAddressStr},
            bytes::{PublicKeyBytes, SecretKeyBytes},
            intent::{PublicTransferIntent, TransferRecipient},
            numeric::BoundaryAmount,
        },
    };

    // --- Fixed, canonical test material ----------------------------------------------------------

    fn fixed_scalar(seed: u8) -> RistrettoSecretKey {
        let mut b = [0u8; 32];
        b[0] = seed;
        RistrettoSecretKey::from_canonical_bytes(&b).expect("low scalar is canonical")
    }

    fn account_secret_bytes() -> SecretKeyBytes {
        SecretKeyBytes::from_bytes(fixed_scalar(11).as_bytes()).unwrap()
    }

    fn keys() -> PublicTransferKeys {
        PublicTransferKeys::new(account_secret_bytes())
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

    fn recipient_pk() -> RistrettoPublicKeyBytes {
        let pk = RistrettoPublicKey::from_secret_key(&fixed_scalar(7));
        RistrettoPublicKeyBytes::from_bytes(pk.as_bytes()).unwrap()
    }

    fn recipient_pk_bytes() -> PublicKeyBytes {
        PublicKeyBytes::from_bytes(recipient_pk().as_bytes()).unwrap()
    }

    /// A resolved-path intent (no explicit inputs ⇒ the want set is derived). Amount above 2^53 µTari
    /// so the u64-safe path is exercised through resolution + seal too.
    fn resolved_intent() -> PublicTransferIntent {
        PublicTransferIntent {
            from_account: ComponentAddressStr::from_internal(&from_component()),
            recipient: TransferRecipient::PublicKey(recipient_pk_bytes()),
            resource_address: ResourceAddressStr::from_internal(&resource()),
            amount: BoundaryAmount::new((1u64 << 53) + 12_345),
            fee: BoundaryAmount::new(2000),
            inputs: vec![],
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
        crate::substate_decode::indexer_substate_json(&SubstateValue::Component(component))
    }

    fn vault_substate_json() -> serde_json::Value {
        let vault = Vault::new(ResourceContainer::public_fungible(resource(), Amount::new(1_000_000)));
        crate::substate_decode::indexer_substate_json(&SubstateValue::Vault(vault))
    }

    fn fetched(id: impl ToString, value: serde_json::Value) -> FetchedSubstate {
        FetchedSubstate {
            substate_id: id.to_string(),
            version: 0,
            substate_value: value,
        }
    }

    /// The full single-round fetched batch that resolves the want set: the from-component (revealing
    /// its vault) + the from-vault (holding the resource).
    fn full_fetched_batch() -> Vec<FetchedSubstate> {
        vec![
            fetched(
                SubstateId::Component(from_component()),
                component_substate_json(&[from_vault_id()]),
            ),
            fetched(SubstateId::Vault(from_vault_id()), vault_substate_json()),
        ]
    }

    /// Drives resolution to a resolved partial for our fixture.
    fn resolved_partial() -> PartialTransaction {
        let (partial, _wants) =
            build_public_transfer_unsigned_with_wants(Network::Esmeralda, &resolved_intent()).unwrap();
        match apply_fetched_substates(partial, &full_fetched_batch()).unwrap() {
            Resolution::Resolved(p) => p,
            Resolution::NeedMore { want_list, .. } => panic!("expected Resolved, got NeedMore({want_list:?})"),
        }
    }

    use crate::types::network::Network;

    // --- Tests -----------------------------------------------------------------------------------

    /// The sealed resolved transaction's signatures must really verify.
    #[test]
    fn resolved_sealed_signatures_verify() {
        let unsigned = resolved_unsigned(resolved_partial()).unwrap();
        let (transaction, _id) = seal_public_transfer(unsigned, &keys()).unwrap();
        assert!(verify_all_signatures(&transaction), "all signatures must verify");
    }

    /// The resolved path produces a valid, decodable transaction (its bytes are non-reproducible).
    #[test]
    fn resolved_path_produces_valid_bytes() {
        let out = seal_and_encode_public_transfer(resolved_partial(), &keys()).unwrap();
        assert!(!out.encoded_transaction.as_bytes().is_empty());
        assert_eq!(out.transaction_id.as_bytes().len(), 32);
    }

    /// Sealing a still-unresolved partial is a `Resolution` error.
    #[test]
    fn sealing_unresolved_partial_is_a_resolution_error() {
        let (partial, _wants) =
            build_public_transfer_unsigned_with_wants(Network::Esmeralda, &resolved_intent()).unwrap();
        // Not resolved yet (no fetch applied).
        let err = seal_and_encode_public_transfer(partial, &keys()).unwrap_err();
        assert_eq!(err.code(), "RESOLUTION");
    }

    /// The canonical sort is order-independent: resolving the same logical input set yields the same
    /// canonical **unsigned** transaction regardless of the order the resolver folds inputs (this is
    /// what makes two resolutions seal over identical input bytes; the signatures themselves are random).
    #[test]
    fn canonical_sort_makes_input_order_irrelevant() {
        // Round 1: fetch only the from-component (reveals the from-vault id).
        let (partial, _w) = build_public_transfer_unsigned_with_wants(Network::Esmeralda, &resolved_intent()).unwrap();
        let r1 = vec![fetched(
            SubstateId::Component(from_component()),
            component_substate_json(&[from_vault_id()]),
        )];
        let partial = match apply_fetched_substates(partial, &r1).unwrap() {
            Resolution::NeedMore { partial, .. } => partial,
            Resolution::Resolved(_) => panic!("expected NeedMore"),
        };
        // Round 2: fetch the from-vault.
        let r2 = vec![fetched(SubstateId::Vault(from_vault_id()), vault_substate_json())];
        let two_round = match apply_fetched_substates(partial, &r2).unwrap() {
            Resolution::Resolved(p) => resolved_unsigned(p).unwrap(),
            Resolution::NeedMore { .. } => panic!("expected Resolved"),
        };

        // Single-round path (component + vault in one batch).
        let single_round = resolved_unsigned(resolved_partial()).unwrap();

        assert_eq!(
            serde_json::to_value(&two_round).unwrap(),
            serde_json::to_value(&single_round).unwrap(),
            "the canonical sort must make multi-round vs single-round resolution produce the identical unsigned tx"
        );
    }
}
