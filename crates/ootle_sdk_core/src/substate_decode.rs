//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Substate decode + account balances: read on-chain state back so a host can verify an operation
//! landed. Three pure, RNG-free, bytes-in → typed-out entry points (the host does the fetch; the
//! core never touches the network):
//!
//! - [`decode_substate`] — any fetched substate value (the indexer's base64 CBOR string) → a typed, kind-tagged
//!   [`DecodedSubstate`].
//! - [`account_balances`] — an account component substate + its already-fetched vault substates → the **revealed**
//!   balance per resource ([`ResourceBalance`]). Vault ids are discovered from the CBOR `ComponentBody.state` (vault
//!   refs are not a direct field), then matched against the supplied vaults.
//! - [`account_balance_wants`] — the account component substate → the canonical `vault_<hex>` ids the host should fetch
//!   to satisfy [`account_balances`] (the same discovery, surfaced as opaque ids).
//!
//! ## Confidential balance
//!
//! A [`Vault`] does not expose a decrypted confidential total. [`Vault::balance`] returns only the
//! revealed/unlocked amount; the confidential value lives encrypted in the commitments. A confidential
//! balance is the sum of separately-scanned, unspent stealth UTXOs filtered by resource — not the
//! vault alone. So [`account_balances`] returns the revealed balance only, and [`DecodedSubstate::Vault`]
//! surfaces the confidential side explicitly as `confidential_commitment_count` and a [`VaultKind`]
//! tag. To verify a confidential delta, scan the relevant UTXOs
//! ([`scan_stealth_substate`](crate::scan_stealth_substate)) and sum client-side.
//!
//! ## u64 safety
//!
//! Balances cross as native `u64` end-to-end, never round-tripped through a float. The engine's
//! [`Amount`] is a fixed-width integer; it is narrowed to `u64` via [`Amount::to_u64_checked`], with
//! an out-of-range value surfaced as a clear error rather than truncated.

use std::str::FromStr;

use serde::{Deserialize, Serialize};
use tari_engine_types::{
    encoded::Encoded,
    substate::{SubstateId, SubstateValue},
};

use crate::{
    FetchedSubstate,
    types::{address::ResourceAddressStr, error::OotleSdkError},
};

/// Whether a vault holds a plain fungible balance or carries confidential (encrypted) commitments.
///
/// Derived from the vault's commitment count: a vault with at least one confidential commitment is
/// [`Confidential`](VaultKind::Confidential) (its spendable value is not the revealed balance),
/// otherwise [`Fungible`](VaultKind::Fungible).
///
/// A `Stealth`-resource vault holds only a revealed amount in the vault itself (its concealed value
/// lives in independent stealth-UTXO substates, scanned separately), so it reports
/// `confidential_commitment_count == 0` and tags as [`Fungible`](VaultKind::Fungible). The
/// `revealed_balance` is the revealed amount only — a stealth total still requires scanning the UTXOs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultKind {
    /// A plain fungible vault: the revealed balance is the full balance.
    Fungible,
    /// A vault carrying confidential commitments: the revealed balance is only the unlocked part;
    /// the confidential value lives encrypted in the commitments.
    Confidential,
}

/// A typed view of a fetched substate, kind-tagged so a host can branch without re-porting the
/// engine's CBOR shape.
///
/// Only the common variants are modelled richly; every other [`SubstateValue`] variant decodes to
/// [`Other`](DecodedSubstate::Other) carrying its kind tag, so a host can recognise it without the
/// core having to model every field.
///
/// The FFI wire shape is `{ "kind": "<variant>", "value": { … } }` (serde adjacently-tagged).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum DecodedSubstate {
    /// An account/component substate. `template_address` and `entity_id` are canonical strings; the
    /// vault ids embedded in the component state are surfaced as `vault_ids` (the same discovery
    /// [`account_balance_wants`] returns).
    Component {
        /// The component's template address (`template_<hex>` canonical string).
        template_address: String,
        /// The component's entity id (canonical string).
        entity_id: String,
        /// The `vault_<hex>` ids referenced by the component's state, in state order.
        vault_ids: Vec<String>,
    },
    /// A vault substate. `revealed_balance` is the revealed/unlocked amount only;
    /// `confidential_commitment_count` and `kind` surface the confidential side explicitly.
    Vault {
        /// The vault's resource address (`resource_<hex>` canonical string).
        resource_address: ResourceAddressStr,
        /// The revealed/unlocked balance, native `u64` (not a confidential total).
        revealed_balance: u64,
        /// The number of confidential commitments the vault holds. `0` ⇒ purely revealed.
        confidential_commitment_count: u64,
        /// Whether the vault is fungible or confidential (derived from the commitment count).
        kind: VaultKind,
    },
    /// A resource substate.
    Resource {
        /// The resource type as a lowercase keyword (`fungible` / `non_fungible` / `confidential` /
        /// `stealth`).
        resource_type: String,
        /// The total supply, native `u64`, when the resource tracks supply; `None` otherwise.
        total_supply: Option<u64>,
    },
    /// Any other substate variant — carries only the kind tag so a host can recognise it without the
    /// core modelling its fields.
    Other {
        /// The [`SubstateValue`] variant name (`non_fungible`, `template`, `utxo`, …).
        variant: String,
    },
}

/// A single resource's revealed balance for an account (`account_balances` element).
///
/// `revealed_balance` is the revealed/unlocked sum across the account's vaults of that resource,
/// native `u64` — not a confidential total.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceBalance {
    /// The resource address (`resource_<hex>` canonical string).
    pub resource_address: ResourceAddressStr,
    /// The revealed balance summed across the account's vaults of this resource, native `u64`.
    pub revealed_balance: u64,
}

/// Narrows an engine [`Amount`](tari_template_lib_types::Amount) to a native `u64`, erroring (never
/// truncating) on a value that exceeds the `u64` range.
fn amount_to_u64(amount: tari_template_lib_types::Amount, context: &str) -> Result<u64, OotleSdkError> {
    amount.to_u64_checked().ok_or_else(|| {
        OotleSdkError::Validation(format!("{context}: balance {amount} does not fit in a u64 (> 2^64-1)"))
    })
}

/// Decodes a substate value as the indexer serves it: a JSON string holding the value's CBOR encoding in base64.
pub(crate) fn decode_indexer_substate_value(value: &serde_json::Value) -> Result<SubstateValue, String> {
    Encoded::<SubstateValue>::deserialize(value)
        .map_err(|e| format!("not a base64 CBOR substate value: {e}"))?
        .decode()
        .map_err(|e| e.to_string())
}

/// `value` as the indexer serves it, for [`decode_indexer_substate_value`].
#[cfg(test)]
pub(crate) fn indexer_substate_json(value: &SubstateValue) -> serde_json::Value {
    serde_json::to_value(Encoded::encode(value).expect("substate value encodes")).expect("base64 serializes")
}

/// Decodes the indexer's substate value into the engine [`SubstateValue`].
///
/// A malformed/unknown substate value surfaces as `Parse` (host-supplied input, not an internal
/// encoding fault).
fn parse_substate_value(value: &serde_json::Value, context: &str) -> Result<SubstateValue, OotleSdkError> {
    decode_indexer_substate_value(value)
        .map_err(|e| OotleSdkError::Parse(format!("undecodable substate value for {context}: {e}")))
}

/// Discovers the canonical `vault_<hex>` ids embedded in a component's CBOR state.
///
/// Vault refs are not a direct field on the component — they live inside
/// [`ComponentBody::state`](tari_engine_types::component::ComponentBody) and are discovered by walking
/// the indexed substate refs.
fn component_vault_id_strings(
    component: &tari_engine_types::component::Component,
    context: &str,
) -> Result<Vec<String>, OotleSdkError> {
    let indexed = component
        .body
        .to_indexed_well_known_types()
        .map_err(|e| OotleSdkError::Parse(format!("failed to index component state for {context}: {e}")))?;
    Ok(indexed
        .vault_ids()
        .iter()
        .map(|v| SubstateId::Vault(*v).to_string())
        .collect())
}

/// Decodes any fetched substate value into a typed, kind-tagged [`DecodedSubstate`].
///
/// `substate_value` is the substate value the indexer returned (its base64 CBOR string), passed verbatim — the same
/// neutral carrier [`FetchedSubstate`](crate::FetchedSubstate) uses. The decode is a pure parse +
/// field map (no RNG), so it is byte-stable.
///
/// For a [`Vault`](DecodedSubstate::Vault) the `revealed_balance` is the revealed/unlocked amount
/// only; the confidential side is surfaced explicitly as `confidential_commitment_count` + `kind` (a
/// `Confidential` vault's spendable value is the sum of unspent stealth UTXOs, scanned separately).
///
/// Errors: a malformed/unknown substate value JSON surfaces as `Parse`; a balance that does not fit a
/// `u64` surfaces as `Validation`.
pub fn decode_substate(substate_value: &serde_json::Value) -> Result<DecodedSubstate, OotleSdkError> {
    let value = parse_substate_value(substate_value, "decode_substate")?;
    decode_substate_value(&value)
}

/// The shared parsed-value → typed mapping (used by [`decode_substate`] and reusable internally).
fn decode_substate_value(value: &SubstateValue) -> Result<DecodedSubstate, OotleSdkError> {
    match value {
        SubstateValue::Component(component) => Ok(DecodedSubstate::Component {
            template_address: component.template_address().to_string(),
            entity_id: component.entity_id().to_string(),
            vault_ids: component_vault_id_strings(component, "decode_substate component")?,
        }),
        SubstateValue::Vault(vault) => {
            let revealed_balance = amount_to_u64(vault.balance(), "decode_substate vault")?;
            let confidential_commitment_count = vault.get_commitment_count();
            let kind = if confidential_commitment_count > 0 {
                VaultKind::Confidential
            } else {
                VaultKind::Fungible
            };
            Ok(DecodedSubstate::Vault {
                resource_address: ResourceAddressStr::from_internal(vault.resource_address()),
                revealed_balance,
                confidential_commitment_count,
                kind,
            })
        },
        SubstateValue::Resource(resource) => {
            let total_supply = match resource.total_supply() {
                Some(amount) => Some(amount_to_u64(amount, "decode_substate resource total_supply")?),
                None => None,
            };
            Ok(DecodedSubstate::Resource {
                resource_type: resource_type_keyword(resource.resource_type()).to_string(),
                total_supply,
            })
        },
        other => Ok(DecodedSubstate::Other {
            variant: substate_variant_name(other).to_string(),
        }),
    }
}

/// The lowercase keyword for a resource type (matches the serde `rename_all = "snake_case"` form).
fn resource_type_keyword(rt: tari_template_lib_types::ResourceType) -> &'static str {
    use tari_template_lib_types::ResourceType::*;
    match rt {
        Fungible => "fungible",
        NonFungible => "non_fungible",
        Confidential => "confidential",
        Stealth => "stealth",
    }
}

/// The lowercase variant name for an unmodelled [`SubstateValue`] (for [`DecodedSubstate::Other`]).
fn substate_variant_name(value: &SubstateValue) -> &'static str {
    match value {
        SubstateValue::Component(_) => "component",
        SubstateValue::Resource(_) => "resource",
        SubstateValue::Vault(_) => "vault",
        SubstateValue::NonFungible(_) => "non_fungible",
        SubstateValue::ClaimedOutputTombstone(_) => "claimed_output_tombstone",
        SubstateValue::TransactionReceipt(_) => "transaction_receipt",
        SubstateValue::Template(_) => "template",
        SubstateValue::ValidatorFeePool(_) => "validator_fee_pool",
        SubstateValue::Utxo(_) => "utxo",
        SubstateValue::ConfidentialOutput(_) => "confidential_output",
    }
}

/// Returns the canonical `vault_<hex>` ids a host should fetch to satisfy [`account_balances`] for an
/// account.
///
/// This is the same component-vault discovery [`account_balances`] does, surfaced as opaque
/// `fetch_ids` so the host can fetch them and feed them back. The host does the fetch; the core only
/// names the ids.
///
/// `account_substate` must be a `Component` substate value as the indexer serves it; anything else (or a malformed
/// value) surfaces as `Validation` / `Parse`.
pub fn account_balance_wants(account_substate: &serde_json::Value) -> Result<Vec<String>, OotleSdkError> {
    let value = parse_substate_value(account_substate, "account_balance_wants")?;
    let component = value
        .component()
        .ok_or_else(|| OotleSdkError::Validation("account_balance_wants: substate is not a component".to_string()))?;
    component_vault_id_strings(component, "account_balance_wants component")
}

/// Computes the revealed balance per resource for an account, summed across its vaults.
///
/// `account_substate` is the account `Component` substate value as the indexer serves it; `vault_substates` are the
/// vault substates the host already fetched, each carrying its **substate id** (the neutral
/// [`FetchedSubstate`] carrier — a [`Vault`](tari_engine_types::vault::Vault) value does not embed its
/// own id, so the id must travel alongside it). The account's vault ids are rediscovered from its CBOR
/// state and matched by id against `vault_substates`: each referenced vault must be present (a missing
/// vault is a `Resolution` error, never a silent zero), and a supplied vault the account does not
/// reference is ignored.
///
/// Balances are the revealed/unlocked amounts only. A confidential total requires scanning the
/// relevant stealth UTXOs separately — see the module docs.
///
/// Errors: a non-component account / a non-vault entry surface as `Validation`; a missing referenced
/// vault surfaces as `Resolution`; a malformed substate JSON surfaces as `Parse`; a non-`u64` balance
/// surfaces as `Validation`.
pub fn account_balances(
    account_substate: &serde_json::Value,
    vault_substates: &[FetchedSubstate],
) -> Result<Vec<ResourceBalance>, OotleSdkError> {
    let account_value = parse_substate_value(account_substate, "account_balances account")?;
    let component = account_value.component().ok_or_else(|| {
        OotleSdkError::Validation("account_balances: account substate is not a component".to_string())
    })?;

    // Discover the account's vault ids from its CBOR state, in state order.
    let wanted_ids = component_vault_id_strings(component, "account_balances component")?;

    // Parse the supplied vault substates, keying each by the substate id the host fetched it under
    // (a Vault value carries no id — it is the substate address), canonicalized so the lookup is
    // robust to id-string formatting.
    let mut supplied: std::collections::HashMap<String, tari_engine_types::vault::Vault> =
        std::collections::HashMap::with_capacity(vault_substates.len());
    for (i, fetched) in vault_substates.iter().enumerate() {
        let value = parse_substate_value(&fetched.substate_value, &format!("account_balances vault[{i}]"))?;
        let vault = value.into_vault().ok_or_else(|| {
            OotleSdkError::Validation(format!(
                "account_balances: vault_substates[{i}] ('{}') is not a vault",
                fetched.substate_id
            ))
        })?;
        let canonical_id = SubstateId::from_str(&fetched.substate_id)
            .map_err(|e| {
                OotleSdkError::Parse(format!(
                    "account_balances: invalid vault substate id '{}': {e}",
                    fetched.substate_id
                ))
            })?
            .to_string();
        supplied.insert(canonical_id, vault);
    }

    // Sum the revealed balance per resource, preserving first-seen order. A discovered vault the host
    // did not supply is a hard error (Resolution), never a silent zero.
    let mut order: Vec<String> = Vec::new();
    let mut totals: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for vault_id in &wanted_ids {
        let vault = supplied.get(vault_id).ok_or_else(|| {
            OotleSdkError::Resolution(format!(
                "account_balances: vault '{vault_id}' referenced by the account was not supplied in vault_substates \
                 (fetch the ids account_balance_wants names)"
            ))
        })?;
        let resource = ResourceAddressStr::from_internal(vault.resource_address());
        let revealed = amount_to_u64(vault.balance(), &format!("account_balances vault '{vault_id}'"))?;
        let key = resource.as_str().to_string();
        let entry = totals.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            0
        });
        *entry = entry.checked_add(revealed).ok_or_else(|| {
            OotleSdkError::Validation(format!(
                "account_balances: revealed balance for resource '{key}' overflows u64"
            ))
        })?;
    }

    Ok(order
        .into_iter()
        .map(|resource_address| ResourceBalance {
            revealed_balance: totals[&resource_address],
            resource_address: ResourceAddressStr(resource_address),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use tari_engine_types::{
        component::{Component, ComponentBody, ComponentHeader},
        resource_container::ResourceContainer,
        substate::SubstateValue,
        vault::Vault,
    };
    use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
    use tari_template_lib_types::{
        Amount,
        EntityId,
        ObjectKey,
        ResourceAddress,
        SubstateOwnerRule,
        VaultId,
        access_rules::ComponentAccessRules,
    };

    use super::*;

    fn resource(seed: u8) -> ResourceAddress {
        ResourceAddress::new(ObjectKey::from_array([seed; ObjectKey::LENGTH]))
    }

    fn vault_id(seed: u8) -> VaultId {
        VaultId::new(ObjectKey::from_array([seed; ObjectKey::LENGTH]))
    }

    /// A fungible vault holding `amount` revealed for `resource`.
    fn fungible_vault(resource: ResourceAddress, amount: u64) -> Vault {
        Vault::new(ResourceContainer::public_fungible(
            resource,
            Amount::new(u128::from(amount)),
        ))
    }

    /// An account component whose CBOR state references `vault_ids`, JSON-encoded as the indexer would
    /// hand it back.
    fn account_substate_json(vault_ids: &[VaultId]) -> serde_json::Value {
        let state = tari_bor::to_value(&vault_ids.to_vec()).expect("encode vault refs");
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

    fn vault_substate_json(vault: &Vault) -> serde_json::Value {
        crate::substate_decode::indexer_substate_json(&SubstateValue::Vault(vault.clone()))
    }

    fn fetched_vault(id: VaultId, vault: &Vault) -> FetchedSubstate {
        FetchedSubstate {
            substate_id: SubstateId::Vault(id).to_string(),
            version: 0,
            substate_value: vault_substate_json(vault),
        }
    }

    #[test]
    fn decode_fungible_vault_reports_revealed_balance() {
        let v = fungible_vault(resource(2), 4_242);
        let decoded = decode_substate(&vault_substate_json(&v)).expect("decode");
        match decoded {
            DecodedSubstate::Vault {
                revealed_balance,
                confidential_commitment_count,
                kind,
                ..
            } => {
                assert_eq!(revealed_balance, 4_242);
                assert_eq!(confidential_commitment_count, 0);
                assert_eq!(kind, VaultKind::Fungible);
            },
            other => panic!("expected Vault, got {other:?}"),
        }
    }

    #[test]
    fn decode_component_lists_vault_ids() {
        let ids = [vault_id(7), vault_id(8)];
        let decoded = decode_substate(&account_substate_json(&ids)).expect("decode");
        match decoded {
            DecodedSubstate::Component { vault_ids, .. } => {
                assert_eq!(vault_ids.len(), 2);
                assert!(vault_ids[0].starts_with("vault_"));
            },
            other => panic!("expected Component, got {other:?}"),
        }
    }

    #[test]
    fn account_balances_sums_revealed_across_vaults_same_resource() {
        let r = resource(9);
        let ids = [vault_id(1), vault_id(2)];
        let v1 = fungible_vault(r, 1_000);
        let v2 = fungible_vault(r, 2_500);
        let account = account_substate_json(&ids);
        let vaults = vec![fetched_vault(ids[0], &v1), fetched_vault(ids[1], &v2)];
        let balances = account_balances(&account, &vaults).expect("balances");
        assert_eq!(balances.len(), 1);
        assert_eq!(balances[0].revealed_balance, 3_500);
    }

    #[test]
    fn account_balances_handles_value_above_2_pow_33() {
        // A balance > 2^33 must survive end-to-end as a native u64 (no float truncation).
        let big: u64 = (1u64 << 33) + 123_456_789;
        let r = resource(5);
        let ids = [vault_id(1)];
        let v = fungible_vault(r, big);
        let account = account_substate_json(&ids);
        let balances = account_balances(&account, &[fetched_vault(ids[0], &v)]).expect("balances");
        assert_eq!(balances[0].revealed_balance, big);
    }

    #[test]
    fn account_balances_missing_vault_is_resolution_error_not_silent_zero() {
        let r = resource(3);
        let ids = [vault_id(1), vault_id(2)];
        let v1 = fungible_vault(r, 1_000);
        // Only supply one of the two referenced vaults.
        let account = account_substate_json(&ids);
        let err = account_balances(&account, &[fetched_vault(ids[0], &v1)]).unwrap_err();
        assert_eq!(err.code(), "RESOLUTION");
    }

    #[test]
    fn account_balances_foreign_vault_is_ignored_but_referenced_still_counted() {
        // A supplied vault that the account does not reference must not be counted.
        let r = resource(4);
        let ids = [vault_id(1)];
        let v_referenced = fungible_vault(r, 500);
        let v_foreign = fungible_vault(resource(50), 9_999);
        let account = account_substate_json(&ids);
        let balances = account_balances(&account, &[
            fetched_vault(ids[0], &v_referenced),
            fetched_vault(vault_id(99), &v_foreign),
        ])
        .expect("balances");
        assert_eq!(balances.len(), 1);
        assert_eq!(balances[0].revealed_balance, 500);
    }

    #[test]
    fn account_balance_wants_returns_account_vault_ids() {
        let ids = [vault_id(11), vault_id(12)];
        let account = account_substate_json(&ids);
        let wants = account_balance_wants(&account).expect("wants");
        assert_eq!(wants.len(), 2);
        assert!(wants.iter().all(|w| w.starts_with("vault_")));
    }

    #[test]
    fn account_balances_non_component_is_validation_error() {
        let v = fungible_vault(resource(2), 1);
        let err = account_balances(&vault_substate_json(&v), &[]).unwrap_err();
        assert_eq!(err.code(), "VALIDATION");
    }

    #[test]
    fn malformed_substate_is_parse_error() {
        let err = decode_substate(&serde_json::json!({ "NotASubstate": 1 })).unwrap_err();
        assert_eq!(err.code(), "PARSE");
    }
}
