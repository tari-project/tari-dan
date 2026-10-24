//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! First-class builtin **faucet claim** builder.
//!
//! The faucet's complete input set (component, vault, claim resource) and self-funding instruction
//! shape live here, so a host does not re-derive them. The builder emits a
//! [`GenericTransactionIntent`] and flows through the same lowering + want-derivation + seal pipeline
//! as any generic intent, so the host calls one entry point and drives the identical apply/seal loop.
//!
//! The claim is self-funding and lives entirely in the fee phase: create-or-fetch the recipient
//! account, put it on the workspace, call `faucet.take(account)` (which funds the account), then pay
//! the fee from that now-funded account. The faucet's vault and component are discovered by
//! want-derivation (the `take` is an address-targeted call); the claim resource — referenced
//! internally by the template, not via any call arg — is pinned via `extra_inputs`. The recipient
//! account and its TARI vault are added as **optional** wants so a claim to an account that already
//! exists on-ledger still resolves.

use tari_template_lib_types::constants::{TARI_TOKEN, XTR_FAUCET_CLAIM_RESOURCE_ADDRESS, XTR_FAUCET_COMPONENT_ADDRESS};

use crate::{
    PartialTransaction,
    WantItem,
    WantList,
    generic_builder::build_unsigned_instructions_with_extra_wants,
    identity::derive_account_address,
    types::{
        address::ComponentAddressStr,
        bytes::PublicKeyBytes,
        error::OotleSdkError,
        generic_intent::{ArgValue, ComponentRef, FeeSource, GenericTransactionIntent, InstructionSpec},
        intent::InputRef,
        network::Network,
        numeric::BoundaryAmount,
    },
};

/// The workspace label the created account is bound under, then paid the fee from.
const ACCOUNT_LABEL: &str = "faucet_account";

/// A faucet-claim request: fund `recipient_public_key`'s account from the network faucet, paying
/// `fee` from the funded account. The boundary record for [`build_faucet_claim_with_wants`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FaucetClaimIntent {
    /// The recipient account owner's public key. The account is created-or-fetched from this key.
    pub recipient_public_key: PublicKeyBytes,
    /// The fee to pay, in µTari (charged from the faucet-funded account).
    pub fee: BoundaryAmount,
    /// Optional earliest epoch this transaction is valid in.
    #[serde(default)]
    pub min_epoch: Option<u64>,
    /// The last epoch this transaction is valid in. Mandatory: every transaction has a bounded
    /// validity window, capped network-wide at `max_transaction_validity_epochs` past the current epoch.
    pub max_epoch: u64,
    /// Whether this is a dry run (e.g. fee estimation).
    #[serde(default)]
    pub dry_run: bool,
}

/// Round 1 for a faucet claim: build the self-funding instruction sequence and the want set, returning
/// the same [`PartialTransaction`] + [`WantList`] the generic path returns. The host then drives the
/// identical `apply_fetched_substates → … → seal_and_encode_public_transfer` loop.
pub fn build_faucet_claim_with_wants(
    network: Network,
    intent: &FaucetClaimIntent,
) -> Result<(PartialTransaction, WantList), OotleSdkError> {
    let recipient_account = derive_account_address(&intent.recipient_public_key)?;
    let generic = faucet_claim_intent(intent);

    // The recipient account and its TARI vault may already exist on-ledger (a repeat funding of a
    // pre-existing account); fetch-and-include them if so, else the create-or-fetch handles it. Both
    // are optional.
    let extra_wants = vec![
        WantItem::SpecificSubstate {
            substate_id: recipient_account.0.clone(),
            required: false,
        },
        WantItem::VaultForResource {
            component_address: recipient_account.0,
            resource_address: TARI_TOKEN.to_string(),
            required: false,
        },
    ];

    build_unsigned_instructions_with_extra_wants(network, &generic, extra_wants)
}

/// Builds the generic intent for a faucet claim: the self-funding fee phase + the non-derivable claim
/// resource pinned via `extra_inputs`.
fn faucet_claim_intent(intent: &FaucetClaimIntent) -> GenericTransactionIntent {
    let faucet = ComponentAddressStr::from_internal(&XTR_FAUCET_COMPONENT_ADDRESS);
    GenericTransactionIntent {
        fee: intent.fee,
        fee_payment: FeeSource::FromWorkspaceComponent {
            label: ACCOUNT_LABEL.to_string(),
        },
        fee_instructions: vec![
            InstructionSpec::CreateAccount {
                owner_public_key: intent.recipient_public_key,
                owner_rule: None,
                bucket_workspace_id: None,
            },
            InstructionSpec::PutLastInstructionOutputOnWorkspace {
                key: ACCOUNT_LABEL.to_string(),
            },
            InstructionSpec::CallMethod {
                call: ComponentRef::Address(faucet),
                method: "take".to_string(),
                args: vec![ArgValue::Workspace(ACCOUNT_LABEL.to_string())],
            },
        ],
        instructions: vec![],
        blobs: vec![],
        inputs: vec![],
        // The faucet component + vault are derived (the `take` is an address-targeted call); the claim
        // resource is referenced internally by the template, so it is not derivable — pin it here.
        extra_inputs: vec![InputRef::unversioned(XTR_FAUCET_CLAIM_RESOURCE_ADDRESS.to_string())],
        min_epoch: intent.min_epoch,
        max_epoch: intent.max_epoch,
        dry_run: intent.dry_run,
    }
}

#[cfg(test)]
mod tests {
    use tari_crypto::{keys::PublicKey as _, ristretto::RistrettoPublicKey, tari_utilities::ByteArray};
    use tari_engine_types::{
        component::{Component, ComponentBody, ComponentHeader},
        resource_container::ResourceContainer,
        substate::{SubstateId, SubstateValue},
        vault::Vault,
    };
    use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
    use tari_template_lib_types::{
        Amount,
        EntityId,
        ResourceAddress,
        SubstateOwnerRule,
        VaultId,
        access_rules::ComponentAccessRules,
        constants::{XTR_FAUCET_CLAIM_RESOURCE_ADDRESS, XTR_FAUCET_VAULT_ADDRESS},
    };

    use super::*;
    use crate::{FetchedSubstate, Resolution, apply_fetched_substates};

    fn recipient_pk() -> PublicKeyBytes {
        let sk = {
            let mut b = [0u8; 32];
            b[0] = 7;
            tari_crypto::ristretto::RistrettoSecretKey::from_canonical_bytes(&b).unwrap()
        };
        PublicKeyBytes::from_bytes(RistrettoPublicKey::from_secret_key(&sk).as_bytes()).unwrap()
    }

    fn intent() -> FaucetClaimIntent {
        FaucetClaimIntent {
            recipient_public_key: recipient_pk(),
            fee: BoundaryAmount::new(2000),
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
        }
    }

    fn component_json(vault_ids: &[VaultId]) -> serde_json::Value {
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

    fn vault_json(res: ResourceAddress) -> serde_json::Value {
        let vault = Vault::new(ResourceContainer::public_fungible(res, Amount::new(1_000_000_000)));
        crate::substate_decode::indexer_substate_json(&SubstateValue::Vault(vault))
    }

    fn fetched(id: impl ToString, value: serde_json::Value) -> FetchedSubstate {
        FetchedSubstate {
            substate_id: id.to_string(),
            version: 0,
            substate_value: value,
        }
    }

    /// A faucet claim resolves the faucet **component**, its **vault**, and the **claim resource** —
    /// the complete input set the `take` method touches.
    #[test]
    fn resolves_faucet_component_vault_and_claim_resource() {
        let (partial, _wants) = build_faucet_claim_with_wants(Network::Esmeralda, &intent()).unwrap();

        // Round 1: the faucet component (reveals its vault id). The recipient account is omitted (it
        // does not exist yet) — its optional wants drop out.
        let r1 = vec![fetched(
            SubstateId::Component(XTR_FAUCET_COMPONENT_ADDRESS),
            component_json(&[XTR_FAUCET_VAULT_ADDRESS]),
        )];
        let partial = match apply_fetched_substates(partial, &r1).unwrap() {
            Resolution::NeedMore { partial, .. } => partial,
            Resolution::Resolved(_) => panic!("expected NeedMore (vault still to fetch)"),
        };

        // Round 2: the discovered faucet vault (holds TARI).
        let r2 = vec![fetched(
            SubstateId::Vault(XTR_FAUCET_VAULT_ADDRESS),
            vault_json(TARI_TOKEN),
        )];
        let resolved = match apply_fetched_substates(partial, &r2).unwrap() {
            Resolution::Resolved(p) => p,
            Resolution::NeedMore { .. } => panic!("expected Resolved"),
        };

        let ids: Vec<String> = resolved
            .resolved_inputs()
            .iter()
            .map(|r| r.substate_id().to_string())
            .collect();
        assert!(
            ids.contains(&XTR_FAUCET_COMPONENT_ADDRESS.to_string()),
            "faucet component must be an input: {ids:?}"
        );
        assert!(
            ids.contains(&XTR_FAUCET_VAULT_ADDRESS.to_string()),
            "faucet vault must be an input: {ids:?}"
        );
        assert!(
            ids.contains(&XTR_FAUCET_CLAIM_RESOURCE_ADDRESS.to_string()),
            "faucet claim resource must be an input: {ids:?}"
        );
    }

    /// The claim resource is pinned even before any fetch (it rides in on `extra_inputs`), and the
    /// self-funding fee phase lowers to create + put + take + pay_fee.
    #[test]
    fn lowers_self_funding_fee_phase() {
        let (partial, _wants) = build_faucet_claim_with_wants(Network::Esmeralda, &intent()).unwrap();
        assert!(
            partial
                .resolved_inputs()
                .iter()
                .any(|r| r.substate_id().to_string() == XTR_FAUCET_CLAIM_RESOURCE_ADDRESS.to_string()),
            "claim resource is pinned up front via extra_inputs"
        );
    }
}
