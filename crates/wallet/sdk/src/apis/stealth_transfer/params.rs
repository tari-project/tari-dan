//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ootle_byte_type::FromByteType;
use ootle_network::Network;
use tari_bor::{Deserialize, Serialize};
use tari_engine_types::crypto::MAX_LAZY_BP_AGG_FACTORS;
use tari_ootle_address::OotleAddress;
use tari_ootle_common_types::Epoch;
use tari_ootle_wallet_crypto::{memo::Memo, pay_to::PayTo, stealth::validated_condition_root};
use tari_template_lib::types::{Amount, ComponentAddress, NonFungibleAddress, ResourceAddress};

use crate::apis::{
    confidential_transfer::UtxoInputSelection,
    stealth_outputs::StealthOutputsApiError,
    stealth_transfer::{StealthOutputToCreate, StealthTransferApiError},
};

#[derive(Debug, Clone)]
pub struct StealthTransferParams {
    /// Parameters related to fee payment
    pub fee_params: TransferFeeParams,
    /// Strategy for input selection
    pub input_selection: UtxoInputSelection,
    pub outputs: Vec<TransferOutput>,
    pub badge_usage: BadgeUsage,
    /// Address of the resource to transfer
    pub resource_address: ResourceAddress,
    /// Fee to lock for the transaction
    pub max_fee: u64,
    /// The last epoch the built transaction may be sequenced in. Mandatory: every transaction
    /// carries a bounded validity window, so the caller decides how long this one stays
    /// submittable.
    pub max_epoch: Epoch,
    /// Run as a dry run, no funds will be transferred if true
    pub is_dry_run: bool,
}

impl StealthTransferParams {
    pub fn validate(&self, network: Network) -> Result<(), StealthTransferApiError> {
        if self.outputs.is_empty() {
            return Err(StealthTransferApiError::InvalidParameter {
                param: "outputs",
                reason: "At least one output must be specified".to_string(),
            });
        }

        let blinded_count = self.outputs.iter().filter(|o| o.blinded_amount > 0).count();
        if blinded_count > MAX_LAZY_BP_AGG_FACTORS {
            return Err(StealthTransferApiError::InvalidParameter {
                param: "outputs",
                reason: format!(
                    "Number of outputs ({}) exceeds maximum allowed ({})",
                    blinded_count, MAX_LAZY_BP_AGG_FACTORS
                ),
            });
        }

        for output in &self.outputs {
            if output.revealed_amount.is_negative() {
                return Err(StealthTransferApiError::InvalidParameter {
                    param: "revealed_output_amount",
                    reason: "Revealed output amount must be non-negative".to_string(),
                });
            }

            if output.blinded_amount == 0 && output.revealed_amount.is_zero() {
                return Err(StealthTransferApiError::InvalidParameter {
                    param: "blinded_output_amount and revealed_output_amount",
                    reason: "At least one of the amounts must be greater than zero".to_string(),
                });
            }

            if output.address.network() != network {
                return Err(StealthTransferApiError::InvalidParameter {
                    param: "destination_address",
                    reason: format!(
                        "Destination address network ({}) does not match wallet network ({})",
                        output.address.network(),
                        network
                    ),
                });
            }

            output
                .address
                .validate()
                .map_err(|e| StealthTransferApiError::InvalidParameter {
                    param: "destination_address",
                    reason: format!("Invalid destination address: {}", e),
                })?;

            output
                .validate_pay_to()
                .map_err(|reason| StealthTransferApiError::InvalidParameter {
                    param: "pay_to",
                    reason,
                })?;
        }

        // A TARI transfer may select inputs covering the outputs and the fee together.
        self.total_output_amount_with_fee(Amount::from(self.max_fee))?;

        if let Some(swap) = &self.fee_params.pay_fee_with_swap &&
            !swap.input_amount.is_positive()
        {
            return Err(StealthTransferApiError::InvalidParameter {
                param: "pay_fee_with_swap.input_amount",
                reason: "The amount of the input resource to swap for the fee must be specified. Use \
                         swap_pools.get_exchange_rate to estimate it."
                    .to_string(),
            });
        }

        Ok(())
    }

    /// The sum of every output's revealed and blinded amounts, or `None` if it exceeds [`Amount::MAX`].
    pub fn total_output_amount(&self) -> Option<Amount> {
        self.outputs
            .iter()
            .try_fold(Amount::zero(), |sum, o| sum.checked_add(o.total_output_amount()?))
    }

    /// The total output amount plus `fee`: what a transfer that also reveals `fee` must select inputs to cover.
    pub fn total_output_amount_with_fee(&self, fee: Amount) -> Result<Amount, StealthTransferApiError> {
        self.total_output_amount()
            .and_then(|total| total.checked_add(fee))
            .ok_or_else(|| StealthTransferApiError::InvalidParameter {
                param: "outputs",
                reason: format!("Total output amount plus a fee of {fee} exceeds the maximum amount"),
            })
    }

    /// The sum of every output's revealed amount, or `None` if it exceeds [`Amount::MAX`].
    pub fn total_revealed_output_amount(&self) -> Option<Amount> {
        Amount::sum_from_positive(self.outputs.iter().map(|o| o.revealed_amount))
    }

    pub fn has_revealed_outputs(&self) -> bool {
        self.outputs.iter().any(|o| o.revealed_amount.is_positive())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransferOutput {
    /// Destination address used to derive the UTXO encryption keys, owner signature and the account in which to
    /// deposit revealed funds
    pub address: OotleAddress,
    /// Amount to spend to a revealed output
    pub revealed_amount: Amount,
    /// Amount to spend to a blinded output
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    pub blinded_amount: u64,
    /// Optional memo to include a memo in the output. This memo is encrypted and can only be read by the recipient.
    pub memo: Option<Memo>,
    pub pay_to: PayTo,
}

impl TransferOutput {
    /// The revealed plus blinded amount, or `None` if it exceeds [`Amount::MAX`].
    pub fn total_output_amount(&self) -> Option<Amount> {
        self.revealed_amount.checked_add(Amount::from(self.blinded_amount))
    }

    /// Checks that this output's `pay_to` intent will be honoured as written, returning the reason it will not.
    ///
    /// `pay_to` gates the spend of the *blinded* output. An output with no blinded amount produces no stealth output
    /// at all — statement construction filters it out — so a gating intent on one would be silently dropped and the
    /// funds deposited unguarded. Reject it rather than mislead the caller.
    ///
    /// For a condition set, the tree is built here so a malformed set is reported against the field the caller
    /// supplied, before any inputs are locked, rather than failing deep in output construction.
    pub fn validate_pay_to(&self) -> Result<(), String> {
        if self.blinded_amount == 0 {
            if matches!(self.pay_to, PayTo::StealthPublicKey) {
                return Ok(());
            }
            return Err(
                "An output with no blinded amount produces no stealth output, so its spend cannot be gated".to_string(),
            );
        }

        if let PayTo::Conditions(conditions) = &self.pay_to {
            validated_condition_root(conditions).map_err(|e| e.to_string())?;
        }

        Ok(())
    }
}

impl<'a> TryFrom<&'a TransferOutput> for StealthOutputToCreate<'a> {
    type Error = StealthOutputsApiError;

    fn try_from(value: &'a TransferOutput) -> Result<Self, Self::Error> {
        Ok(Self {
            owner_address: value.address.try_from_byte_type().map_err(|e| {
                StealthOutputsApiError::InvalidParameter {
                    param: "destination_address",
                    reason: format!("Invalid destination address: {}", e),
                }
            })?,
            amount: value.blinded_amount,
            memo: value.memo.as_ref(),
            pay_to: value.pay_to.clone(),
        })
    }
}
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub enum BadgeUsage {
    /// Do not use a badge
    #[default]
    None,
    /// Use a resource as a badge
    Resource(ResourceAddress),
    /// Use a specific NFT as a badge
    NonFungible(NonFungibleAddress),
    /// Use a specified amount of resource as a badge
    AmountOfResource { resource: ResourceAddress, amount: Amount },
}

impl BadgeUsage {
    pub fn resource_address(&self) -> Option<&ResourceAddress> {
        match self {
            BadgeUsage::None => None,
            BadgeUsage::Resource(addr) => Some(addr),
            BadgeUsage::NonFungible(nft_addr) => Some(nft_addr.resource_address()),
            BadgeUsage::AmountOfResource { resource, .. } => Some(resource),
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, BadgeUsage::None)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct TransferFeeParams {
    pub input_selection: UtxoInputSelection,
    pub pay_fee_with_swap: Option<PayFeeWithSwapParams>,
}

impl TransferFeeParams {
    pub fn new(input_selection: UtxoInputSelection) -> Self {
        Self {
            input_selection,
            pay_fee_with_swap: None,
        }
    }

    pub fn with_pay_fee_with_swap(mut self, params: PayFeeWithSwapParams) -> Self {
        self.pay_fee_with_swap = Some(params);
        self
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct PayFeeWithSwapParams {
    pub pool_address: ComponentAddress,
    pub input_resource: ResourceAddress,
    /// The exact amount of `input_resource` withdrawn and swapped to pay the fee. Must be positive: it is the
    /// caller's bound on what the fee payment spends of the input resource.
    pub input_amount: Amount,
    pub min_xtr_output_amount: Amount,
}

#[cfg(test)]
mod tests {
    use ootle_byte_type::ToByteType;
    use tari_crypto::{keys::PublicKey as _, ristretto::RistrettoPublicKey};
    use tari_engine_types::limits::STEALTH_LIMITS;
    use tari_template_lib::types::{
        constants::STEALTH_TARI_RESOURCE_ADDRESS,
        stealth::{AtomicCondition, BuiltinPredicate, SpendCondition},
    };

    use super::*;

    const NETWORK: Network = Network::LocalNet;

    fn address() -> OotleAddress {
        let (_, view_only) = RistrettoPublicKey::random_keypair(&mut rand::rng());
        let (_, account) = RistrettoPublicKey::random_keypair(&mut rand::rng());
        OotleAddress::new(NETWORK, view_only.to_byte_type(), account.to_byte_type())
    }

    /// A single-leaf condition, distinct per `epoch`.
    fn a_condition(epoch: u64) -> SpendCondition {
        SpendCondition::all([AtomicCondition::Builtin(BuiltinPredicate::AfterEpoch(epoch))])
    }

    /// A single output of `blinded_amount` blinded plus `revealed_amount` revealed, paying to `pay_to`.
    fn params_paying_to(blinded_amount: u64, revealed_amount: u64, pay_to: PayTo) -> StealthTransferParams {
        StealthTransferParams {
            max_epoch: Epoch(1),
            fee_params: TransferFeeParams::new(UtxoInputSelection::PreferConfidential),
            input_selection: UtxoInputSelection::PreferConfidential,
            outputs: vec![TransferOutput {
                address: address(),
                revealed_amount: Amount::from(revealed_amount),
                blinded_amount,
                memo: None,
                pay_to,
            }],
            badge_usage: BadgeUsage::None,
            resource_address: STEALTH_TARI_RESOURCE_ADDRESS,
            max_fee: 1000,
            is_dry_run: false,
        }
    }

    fn blinded_paying_to(pay_to: PayTo) -> StealthTransferParams {
        params_paying_to(100, 0, pay_to)
    }

    fn assert_rejects(params: StealthTransferParams) {
        let err = params
            .validate(NETWORK)
            .expect_err("a pay_to that will not be honoured as written must be rejected");
        assert!(
            matches!(err, StealthTransferApiError::InvalidParameter { param: "pay_to", .. }),
            "expected an InvalidParameter naming pay_to, got: {err}"
        );
    }

    fn assert_rejects_pay_to(pay_to: PayTo) {
        assert_rejects(blinded_paying_to(pay_to));
    }

    #[test]
    fn rejects_an_empty_condition_set() {
        assert_rejects_pay_to(PayTo::Conditions(vec![]));
    }

    #[test]
    fn rejects_duplicate_condition_leaves() {
        let condition = a_condition(1);
        assert_rejects_pay_to(PayTo::Conditions(vec![condition.clone(), condition]));
    }

    /// A leaf the engine refuses to evaluate is a spend path that can never be taken. A tree built only from such
    /// leaves is well formed but unspendable, so it must not reach an output.
    #[test]
    fn rejects_a_structurally_inadmissible_leaf() {
        assert_rejects_pay_to(PayTo::Conditions(vec![SpendCondition::all([])]));
        assert_rejects_pay_to(PayTo::Conditions(vec![SpendCondition::all(vec![
            AtomicCondition::Builtin(BuiltinPredicate::AfterEpoch(1));
            STEALTH_LIMITS.max_conditions_per_conjunction + 1
        ])]));
    }

    /// `pay_to` gates the blinded output, which a revealed-only transfer never produces. Silently depositing the
    /// funds unguarded would give the caller the opposite of what they asked for.
    #[test]
    fn rejects_a_gated_output_with_no_blinded_amount() {
        assert_rejects(params_paying_to(0, 100, PayTo::Conditions(vec![a_condition(1)])));
    }

    #[test]
    fn accepts_an_ungated_output_with_no_blinded_amount() {
        params_paying_to(0, 100, PayTo::StealthPublicKey)
            .validate(NETWORK)
            .expect("a revealed-only output needs no spend gating");
    }

    fn params_paying_fee_with_swap(input_amount: u64) -> StealthTransferParams {
        let mut params = params_paying_to(100, 0, PayTo::StealthPublicKey);
        params.fee_params = params.fee_params.with_pay_fee_with_swap(PayFeeWithSwapParams {
            pool_address: ComponentAddress::from_array([1u8; 32]),
            input_resource: STEALTH_TARI_RESOURCE_ADDRESS,
            input_amount: Amount::from(input_amount),
            min_xtr_output_amount: Amount::from(1000u64),
        });
        params
    }

    /// The swap input is the amount of the caller's token that leaves the account to pay the fee. It must be a figure
    /// the caller chose, so a zero amount is rejected.
    #[test]
    fn rejects_a_fee_swap_without_an_input_amount() {
        let err = params_paying_fee_with_swap(0)
            .validate(NETWORK)
            .expect_err("a fee swap must state its input amount");
        assert!(
            matches!(err, StealthTransferApiError::InvalidParameter {
                param: "pay_fee_with_swap.input_amount",
                ..
            }),
            "expected an InvalidParameter naming pay_fee_with_swap.input_amount, got: {err}"
        );
    }

    #[test]
    fn accepts_a_fee_swap_with_a_positive_input_amount() {
        params_paying_fee_with_swap(2100)
            .validate(NETWORK)
            .expect("a fee swap with an explicit input amount is valid");
    }

    fn params_with_outputs(amounts: &[(Amount, u64)], max_fee: u64) -> StealthTransferParams {
        StealthTransferParams {
            outputs: amounts
                .iter()
                .map(|&(revealed_amount, blinded_amount)| TransferOutput {
                    address: address(),
                    revealed_amount,
                    blinded_amount,
                    memo: None,
                    pay_to: PayTo::StealthPublicKey,
                })
                .collect(),
            max_fee,
            ..params_paying_to(100, 0, PayTo::StealthPublicKey)
        }
    }

    fn assert_rejects_output_total(params: StealthTransferParams) {
        let err = params
            .validate(NETWORK)
            .expect_err("outputs whose total exceeds the maximum amount must be rejected");
        assert!(
            matches!(err, StealthTransferApiError::InvalidParameter { param: "outputs", .. }),
            "expected an InvalidParameter naming outputs, got: {err}"
        );
    }

    #[test]
    fn rejects_outputs_whose_total_exceeds_the_maximum_amount() {
        assert_rejects_output_total(params_with_outputs(&[(Amount::MAX, 0), (Amount::ONE, 0)], 0));
    }

    #[test]
    fn rejects_an_output_whose_revealed_and_blinded_amounts_exceed_the_maximum_amount() {
        assert_rejects_output_total(params_with_outputs(&[(Amount::MAX, 1)], 0));
    }

    #[test]
    fn rejects_outputs_whose_total_plus_the_fee_exceeds_the_maximum_amount() {
        assert_rejects_output_total(params_with_outputs(&[(Amount::MAX, 0)], 1));
    }

    #[test]
    fn accepts_outputs_totalling_the_maximum_amount() {
        let params = params_with_outputs(&[(Amount::MAX - Amount::from(3u64), 1), (Amount::ONE, 1)], 0);
        params
            .validate(NETWORK)
            .expect("a total of exactly the maximum amount is representable");
        assert_eq!(params.total_output_amount(), Some(Amount::MAX));
    }

    #[test]
    fn accepts_a_well_formed_condition_set() {
        blinded_paying_to(PayTo::Conditions(vec![a_condition(1), a_condition(2)]))
            .validate(NETWORK)
            .expect("distinct condition leaves form a tree");
    }
}
