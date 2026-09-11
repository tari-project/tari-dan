//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Public-transfer **intent** records — a developer-facing description, not an instruction list.
//!
//! The intent (plus an explicit key bundle) is converted into builder calls and a deterministic
//! seal. We model the intent and provide the conversions it needs: the explicit input set maps to
//! [`SubstateRequirement`]s, the recipient is either a public key or a component address, and epochs
//! are plain `Option<u64>`.

use std::str::FromStr;

use serde::{Deserialize, Serialize};
use tari_engine_types::substate::SubstateId;
use tari_ootle_common_types::SubstateRequirement;

use crate::types::{
    address::{ComponentAddressStr, ResourceAddressStr},
    bytes::PublicKeyBytes,
    error::OotleSdkError,
    numeric::BoundaryAmount,
};

/// One explicit input: a substate-id string (`<prefix>_<hex>`) plus an optional version.
///
/// Mirrors [`SubstateRequirement`] at the boundary. Uses an explicit input set (no automatic input
/// resolution).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputRef {
    /// The canonical substate id, e.g. `component_<hex>` / `resource_<hex>`.
    pub substate_id: String,
    /// The optional explicit version.
    pub version: Option<u64>,
}

impl InputRef {
    /// Builds an unversioned input ref.
    pub fn unversioned(substate_id: impl Into<String>) -> Self {
        Self {
            substate_id: substate_id.into(),
            version: None,
        }
    }

    /// Builds a versioned input ref.
    pub fn versioned(substate_id: impl Into<String>, version: u64) -> Self {
        Self {
            substate_id: substate_id.into(),
            version: Some(version),
        }
    }

    /// Converts to the internal [`SubstateRequirement`], parsing the substate id.
    pub fn to_internal(&self) -> Result<SubstateRequirement, OotleSdkError> {
        let id = SubstateId::from_str(&self.substate_id)
            .map_err(|e| OotleSdkError::Parse(format!("invalid substate id '{}': {e}", self.substate_id)))?;
        Ok(SubstateRequirement::new(id, self.version))
    }

    /// Builds from an internal [`SubstateRequirement`].
    pub fn from_internal(req: &SubstateRequirement) -> Self {
        Self {
            substate_id: req.substate_id().to_string(),
            version: req.version(),
        }
    }
}

/// The recipient of a public transfer: either a raw account public key or an existing account
/// component address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferRecipient {
    /// The recipient's account public key (the account may not yet exist on-ledger).
    PublicKey(PublicKeyBytes),
    /// An existing recipient account component.
    Account(ComponentAddressStr),
}

/// A public-transfer intent: the minimal developer-facing description of "send `amount` of
/// `resource` from `from_account` to `recipient`, paying `fee`".
///
/// This is intent only — it is turned into the concrete instruction sequence downstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicTransferIntent {
    /// The sender (source) account component.
    pub from_account: ComponentAddressStr,
    /// The recipient (public key or account component).
    pub recipient: TransferRecipient,
    /// The resource being transferred.
    pub resource_address: ResourceAddressStr,
    /// The amount to transfer, in µTari.
    pub amount: BoundaryAmount,
    /// The fee to pay, in µTari.
    pub fee: BoundaryAmount,
    /// The explicit input set (inputs are not resolved here).
    pub inputs: Vec<InputRef>,
    /// Optional earliest epoch this transaction is valid in.
    pub min_epoch: Option<u64>,
    /// The last epoch this transaction is valid in. Mandatory: every transaction has a bounded
    /// validity window, capped network-wide at `max_transaction_validity_epochs` past the current epoch.
    pub max_epoch: u64,
    /// Whether this is a dry run. The core sets `is_seal_signer_authorized` itself — it is
    /// intentionally **not** part of the intent.
    pub dry_run: bool,
}

impl PublicTransferIntent {
    /// Converts the explicit input set to internal [`SubstateRequirement`]s. Used when lowering the
    /// intent to builder calls.
    pub fn inputs_to_internal(&self) -> Result<Vec<SubstateRequirement>, OotleSdkError> {
        self.inputs.iter().map(InputRef::to_internal).collect()
    }
}

#[cfg(test)]
mod tests {
    use tari_template_lib_types::{ComponentAddress, ObjectKey, ResourceAddress};

    use super::*;

    fn component_str() -> String {
        ComponentAddress::new(ObjectKey::from_array([0xaa; ObjectKey::LENGTH])).to_string()
    }

    fn resource_str() -> String {
        ResourceAddress::new(ObjectKey::from_array([0xbb; ObjectKey::LENGTH])).to_string()
    }

    #[test]
    fn input_ref_round_trips_through_internal() {
        let r = InputRef::versioned(component_str(), 3);
        let internal = r.to_internal().unwrap();
        assert_eq!(internal.version(), Some(3));
        assert_eq!(InputRef::from_internal(&internal), r);

        let u = InputRef::unversioned(resource_str());
        let internal = u.to_internal().unwrap();
        assert_eq!(internal.version(), None);
        assert_eq!(InputRef::from_internal(&internal), u);
    }

    #[test]
    fn input_ref_rejects_garbage_id() {
        assert_eq!(
            InputRef::unversioned("not-an-id").to_internal().unwrap_err().code(),
            "PARSE"
        );
    }

    #[test]
    fn intent_inputs_convert_smoke() {
        let intent = PublicTransferIntent {
            from_account: ComponentAddressStr::parse(component_str()).unwrap(),
            recipient: TransferRecipient::PublicKey(PublicKeyBytes::from_array([1u8; 32])),
            resource_address: ResourceAddressStr::parse(resource_str()).unwrap(),
            amount: BoundaryAmount::new((1u64 << 53) + 1),
            fee: BoundaryAmount::new(1000),
            inputs: vec![
                InputRef::versioned(component_str(), 0),
                InputRef::unversioned(resource_str()),
            ],
            min_epoch: Some(10),
            max_epoch: 1,
            dry_run: false,
        };
        let reqs = intent.inputs_to_internal().unwrap();
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].version(), Some(0));
        assert_eq!(reqs[1].version(), None);
        // The amount survives the µTari boundary above 2^53.
        assert_eq!(intent.amount.to_internal().to_u128(), u128::from((1u64 << 53) + 1));
    }

    #[test]
    fn intent_serde_round_trips() {
        let intent = PublicTransferIntent {
            from_account: ComponentAddressStr::parse(component_str()).unwrap(),
            recipient: TransferRecipient::Account(ComponentAddressStr::parse(component_str()).unwrap()),
            resource_address: ResourceAddressStr::parse(resource_str()).unwrap(),
            amount: BoundaryAmount::new(5_000_000),
            fee: BoundaryAmount::new(2000),
            inputs: vec![InputRef::unversioned(resource_str())],
            min_epoch: None,
            max_epoch: 99,
            dry_run: true,
        };
        let json = serde_json::to_string(&intent).unwrap();
        let back: PublicTransferIntent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, intent);
    }
}
