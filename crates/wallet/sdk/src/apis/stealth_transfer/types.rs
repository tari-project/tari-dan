//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::stealth::MergedStealthTransferShape;
use tari_ootle_address::RistrettoOotleAddress;
use tari_ootle_transaction::UnsignedTransaction;
use tari_ootle_wallet_crypto::{StealthInputWitness, memo::Memo, pay_to::PayTo};
use tari_template_lib::types::{Amount, ComponentAddress, VaultId};

use crate::models::{InputSpendData, StealthUtxoSpendKeyId, WalletPublicKey};

pub struct StealthTransferOutput {
    pub transaction: UnsignedTransaction,
    pub fee_inputs: InputsToSpend,
    pub transfer_inputs: InputsToSpend,
    pub utxo_spend_keys: Vec<StealthUtxoSpendKeyId>,
    pub additional_signer: Option<WalletPublicKey>,
    pub main_signer: WalletPublicKey,
    /// What a static fee estimate needs to price this build, when its shape is one the estimate
    /// covers. `None` for every other shape — see [`StaticallyPricedShape`].
    pub statically_priced_shape: Option<StaticallyPricedShape>,
}

/// The counts a [`MergedStealthTransferShape`] is built from, recorded while the transfer is built.
///
/// Present only when the build is the shape the estimate models: one stealth transfer statement that
/// both moves the funds and reveals the fee, with no badge proof, no revealed input to withdraw and
/// no recipient owed revealed funds. Any of those brings instructions the estimate does not price —
/// a template load and the WASM execution behind it — so the estimate must not be applied to them.
///
/// The transaction's weight is missing because it is not known until the transaction is signed and
/// sealed, and what its declared inputs weigh because measuring them takes the network:
/// [`Self::with_transaction`] completes the shape once both are known.
#[derive(Debug, Clone, Copy)]
pub struct StaticallyPricedShape {
    num_inputs: usize,
    num_outputs: usize,
    persisted_output_bytes: usize,
    has_view_key: bool,
}

impl StaticallyPricedShape {
    pub(super) fn new(
        num_inputs: usize,
        num_outputs: usize,
        persisted_output_bytes: usize,
        has_view_key: bool,
    ) -> Self {
        Self {
            num_inputs,
            num_outputs,
            persisted_output_bytes,
            has_view_key,
        }
    }

    /// The shape a fee estimate prices, given the weight of the sealed transaction this build
    /// produced, the encoded bytes of its declared inputs and the other shard groups they are pledged
    /// to.
    pub fn with_transaction(
        &self,
        transaction_weight: u64,
        input_bytes: u64,
        input_fanout: u32,
    ) -> MergedStealthTransferShape {
        MergedStealthTransferShape {
            num_inputs: self.num_inputs,
            num_outputs: self.num_outputs,
            persisted_output_bytes: self.persisted_output_bytes,
            has_view_key: self.has_view_key,
            transaction_weight,
            input_bytes,
            input_fanout,
        }
    }
}

#[derive(Debug)]
pub struct UnblindedInputToSpend {
    pub witness: StealthInputWitness,
}

impl UnblindedInputToSpend {
    pub fn value(&self) -> u64 {
        self.witness.mask_and_value.value
    }
}

#[derive(Debug, Clone)]
pub struct StealthOutputToCreate<'a> {
    pub owner_address: RistrettoOotleAddress,
    pub pay_to: PayTo,
    pub amount: u64,
    pub memo: Option<&'a Memo>,
}

#[derive(Debug)]
pub struct InputsToSpend {
    pub inputs: Vec<InputSpendData>,
    pub revealed: Amount,
}

impl InputsToSpend {
    /// No funds locked. Used for the fee half of a transfer whose fee is sourced by the transfer
    /// statement itself rather than by a statement of its own.
    pub fn empty() -> Self {
        Self {
            inputs: vec![],
            revealed: Amount::zero(),
        }
    }

    pub fn inputs_iter(&self) -> impl Iterator<Item = &InputSpendData> + '_ {
        self.inputs.iter()
    }

    pub fn total_amount(&self) -> Amount {
        self.total_stealth_input_amount() + self.revealed
    }

    pub fn total_stealth_input_amount(&self) -> Amount {
        self.inputs.iter().map(|i| Amount::from(i.value)).sum()
    }
}

pub struct AccountDetails {
    pub address: ComponentAddress,
    pub vaults: Vec<VaultId>,
    pub exists: bool,
}
