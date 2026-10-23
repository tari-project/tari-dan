// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use log::warn;
use tari_engine_types::limits::STEALTH_LIMITS;
use tari_ootle_transaction::{Instruction, Transaction};

use crate::{TransactionValidationError, Validator};

const LOG_TARGET: &str = "tari::ootle::mempool::validators::stealth_limits";

/// Rejects transactions whose stealth-transfer work exceeds the per-transfer or per-transaction caps in
/// `STEALTH_LIMITS`.
///
/// Verifying a stealth transfer is native, unmetered crypto (a bulletproof range proof plus an ElGamal proof per
/// output), so without a per-transaction cap a single transaction could stack enough verification to push the
/// proposing leader past the block time. The engine enforces the same per-transfer and per-transaction caps during
/// execution; this mirrors them to reject doomed transactions at ingress, before they are gossiped, stored and
/// executed.
///
/// `max_fee_intent_transfers` is mirrored here too, over the fee instructions alone. The engine counts transfers a
/// template performs as well, which an instruction count cannot see, so this catches the common case at ingress while
/// the engine remains authoritative.
#[derive(Debug, Clone, Default)]
pub struct StealthTransactionLimitsValidator;

impl StealthTransactionLimitsValidator {
    pub fn new() -> Self {
        Self
    }
}

impl Validator<Transaction> for StealthTransactionLimitsValidator {
    type Context = ();
    type Error = TransactionValidationError;

    fn validate(&self, _context: &(), transaction: &Transaction) -> Result<(), Self::Error> {
        let mut transfers = 0usize;
        let mut fee_intent_transfers = 0usize;
        let mut inputs = 0usize;
        let mut outputs = 0usize;

        for (is_fee_intent, instruction) in transaction
            .fee_instructions()
            .iter()
            .map(|i| (true, i))
            .chain(transaction.instructions().iter().map(|i| (false, i)))
        {
            if let Instruction::StealthTransfer { statement, .. } = instruction {
                let transfer_inputs = statement.inputs_statement.inputs.len();
                let transfer_outputs = statement.outputs_statement.outputs.len();
                // Mirror the engine's per-transfer limits (check_stealth_transfer_limits) so a single oversized
                // transfer is rejected at ingress rather than aborting during execution.
                self.check(
                    "per-transfer inputs",
                    transfer_inputs,
                    STEALTH_LIMITS.max_inputs,
                    transaction,
                )?;
                self.check(
                    "per-transfer outputs",
                    transfer_outputs,
                    STEALTH_LIMITS.max_outputs,
                    transaction,
                )?;
                transfers += 1;
                if is_fee_intent {
                    fee_intent_transfers += 1;
                }
                inputs += transfer_inputs;
                outputs += transfer_outputs;
            }
        }

        self.check(
            "fee intent transfers",
            fee_intent_transfers,
            STEALTH_LIMITS.max_fee_intent_transfers,
            transaction,
        )?;
        self.check(
            "transfers",
            transfers,
            STEALTH_LIMITS.max_transfers_per_transaction,
            transaction,
        )?;
        self.check(
            "inputs",
            inputs,
            STEALTH_LIMITS.max_total_inputs_per_transaction,
            transaction,
        )?;
        self.check(
            "outputs",
            outputs,
            STEALTH_LIMITS.max_total_outputs_per_transaction,
            transaction,
        )?;
        Ok(())
    }
}

impl StealthTransactionLimitsValidator {
    fn check(
        &self,
        limit: &'static str,
        actual: usize,
        max: usize,
        transaction: &Transaction,
    ) -> Result<(), TransactionValidationError> {
        if actual > max {
            let transaction_id = transaction.calculate_id();
            warn!(
                target: LOG_TARGET,
                "StealthTransactionLimitsValidator - FAIL: {transaction_id} stealth {limit} {actual} exceeds maximum {max}"
            );
            return Err(TransactionValidationError::ExceedsStealthTransactionLimit {
                transaction_id,
                limit,
                max,
                actual,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use indexmap::IndexSet;
    use tari_ootle_common_types::Epoch;
    use tari_ootle_transaction::{
        Network,
        ResourceAddressRef,
        Sr25519SignatureBytes,
        TransactionSealSignature,
        TransactionSignature,
        UnsealedTransactionV1,
        UnsignedTransactionV1,
    };
    use tari_template_lib::{
        prelude::SpendAuthorization,
        types::{
            Amount,
            EncryptedData,
            crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes, UtxoTag},
            stealth::{StealthInput, StealthTransferStatement, StealthUnspentOutput, UnspentOutput},
        },
    };

    use super::*;

    fn dummy_commitment() -> PedersenCommitmentBytes {
        PedersenCommitmentBytes::from_bytes(&[0u8; 32]).unwrap()
    }

    fn dummy_output() -> StealthUnspentOutput {
        StealthUnspentOutput {
            output: UnspentOutput {
                commitment: dummy_commitment(),
                sender_public_nonce: RistrettoPublicKeyBytes::zero(),
                encrypted_data: EncryptedData::try_from(vec![0u8; EncryptedData::min_size()]).unwrap(),
                minimum_value_promise: 0,
                viewable_balance_proof: None,
            },
            auth: SpendAuthorization::Key(RistrettoPublicKeyBytes::zero()),
            tag: UtxoTag::new(0),
        }
    }

    /// A stealth transfer statement with the given input/output counts. Only the counts matter to this validator, so
    /// the commitments and proofs are dummies.
    fn statement(n_inputs: usize, n_outputs: usize) -> StealthTransferStatement {
        // A non-zero revealed amount keeps the statement constructor happy; only the input/output counts matter here.
        let mut stmt =
            StealthTransferStatement::revealed_only(Amount::new(1), Amount::new(1), RistrettoPublicKeyBytes::zero());
        stmt.inputs_statement.inputs = (0..n_inputs).map(|_| StealthInput::new(dummy_commitment())).collect();
        stmt.outputs_statement.outputs = (0..n_outputs).map(|_| dummy_output()).collect();
        stmt
    }

    fn transfer_instructions(statements: Vec<StealthTransferStatement>) -> Vec<Instruction> {
        statements
            .into_iter()
            .map(|statement| Instruction::StealthTransfer {
                resource_address_ref: ResourceAddressRef::from(0u16),
                statement,
                revealed_input_bucket: None,
            })
            .collect()
    }

    fn tx_with_stealth_transfers(statements: Vec<StealthTransferStatement>) -> Transaction {
        tx_with_intents(vec![], statements)
    }

    fn tx_with_intents(
        fee_statements: Vec<StealthTransferStatement>,
        main_statements: Vec<StealthTransferStatement>,
    ) -> Transaction {
        Transaction::new(
            UnsealedTransactionV1::new(
                UnsignedTransactionV1::new(
                    Network::LocalNet.as_byte(),
                    transfer_instructions(fee_statements),
                    transfer_instructions(main_statements),
                    IndexSet::new(),
                    None,
                    Epoch(1),
                    false,
                ),
                vec![TransactionSignature::new(
                    RistrettoPublicKeyBytes::zero(),
                    Sr25519SignatureBytes::zero(),
                )],
            )
            .into(),
            TransactionSealSignature::new(RistrettoPublicKeyBytes::zero(), Sr25519SignatureBytes::zero()),
        )
    }

    #[test]
    fn accepts_transaction_at_the_caps() {
        let limits = STEALTH_LIMITS;
        // Distribute the total input/output caps evenly across transfers so the transaction sits exactly on both
        // totals while every transfer also respects the per-transfer limits, padded with empty transfers up to the
        // transfer cap.
        const N: usize = 32;
        let inputs_each = limits.max_total_inputs_per_transaction / N;
        let outputs_each = limits.max_total_outputs_per_transaction / N;
        assert!(inputs_each <= limits.max_inputs && outputs_each <= limits.max_outputs);
        assert_eq!(inputs_each * N, limits.max_total_inputs_per_transaction);
        assert_eq!(outputs_each * N, limits.max_total_outputs_per_transaction);
        let mut statements = (0..N).map(|_| statement(inputs_each, outputs_each)).collect::<Vec<_>>();
        statements.extend((statements.len()..limits.max_transfers_per_transaction).map(|_| statement(0, 0)));
        let tx = tx_with_stealth_transfers(statements);
        StealthTransactionLimitsValidator::new().validate(&(), &tx).unwrap();
    }

    #[test]
    fn rejects_transfer_exceeding_per_transfer_outputs() {
        let tx = tx_with_stealth_transfers(vec![statement(0, STEALTH_LIMITS.max_outputs + 1)]);
        let err = StealthTransactionLimitsValidator::new().validate(&(), &tx).unwrap_err();
        assert!(matches!(
            err,
            TransactionValidationError::ExceedsStealthTransactionLimit {
                limit: "per-transfer outputs",
                ..
            }
        ));
    }

    #[test]
    fn rejects_transfer_exceeding_per_transfer_inputs() {
        let tx = tx_with_stealth_transfers(vec![statement(STEALTH_LIMITS.max_inputs + 1, 0)]);
        let err = StealthTransactionLimitsValidator::new().validate(&(), &tx).unwrap_err();
        assert!(matches!(
            err,
            TransactionValidationError::ExceedsStealthTransactionLimit {
                limit: "per-transfer inputs",
                ..
            }
        ));
    }

    #[test]
    fn rejects_too_many_transfers() {
        let n = STEALTH_LIMITS.max_transfers_per_transaction + 1;
        let tx = tx_with_stealth_transfers((0..n).map(|_| statement(0, 0)).collect());
        let err = StealthTransactionLimitsValidator::new().validate(&(), &tx).unwrap_err();
        assert!(matches!(
            err,
            TransactionValidationError::ExceedsStealthTransactionLimit { limit: "transfers", .. }
        ));
    }

    /// One fee-sourcing transfer is admitted, and the cap applies to the fee intent alone — the main intent may carry
    /// as many transfers as the per-transaction caps allow.
    #[test]
    fn admits_one_fee_intent_transfer_alongside_several_main_intent_transfers() {
        let tx = tx_with_intents(vec![statement(16, 1)], (0..8).map(|_| statement(4, 2)).collect());
        StealthTransactionLimitsValidator::new().validate(&(), &tx).unwrap();
    }

    #[test]
    fn rejects_a_second_fee_intent_transfer() {
        let n = STEALTH_LIMITS.max_fee_intent_transfers + 1;
        let tx = tx_with_intents((0..n).map(|_| statement(1, 1)).collect(), vec![]);
        let err = StealthTransactionLimitsValidator::new().validate(&(), &tx).unwrap_err();
        assert!(matches!(
            err,
            TransactionValidationError::ExceedsStealthTransactionLimit {
                limit: "fee intent transfers",
                ..
            }
        ));
    }

    #[test]
    fn rejects_too_many_total_inputs_summed_across_transfers() {
        // Two transfers, each within the per-transfer input limit, that together exceed the per-transaction total —
        // proving the validator sums across the transaction rather than catching it per-transfer.
        let per = STEALTH_LIMITS.max_total_inputs_per_transaction / 2 + 1;
        assert!(per <= STEALTH_LIMITS.max_inputs);
        let tx = tx_with_stealth_transfers(vec![statement(per, 0), statement(per, 0)]);
        let err = StealthTransactionLimitsValidator::new().validate(&(), &tx).unwrap_err();
        assert!(matches!(
            err,
            TransactionValidationError::ExceedsStealthTransactionLimit { limit: "inputs", .. }
        ));
    }

    #[test]
    fn rejects_too_many_total_outputs_summed_across_transfers() {
        // Spread the over-cap output total across transfers that each respect the per-transfer output limit, proving
        // the validator sums across the transaction.
        let n_transfers = STEALTH_LIMITS.max_total_outputs_per_transaction / STEALTH_LIMITS.max_outputs + 1;
        let tx = tx_with_stealth_transfers(
            (0..n_transfers)
                .map(|_| statement(0, STEALTH_LIMITS.max_outputs))
                .collect(),
        );
        let err = StealthTransactionLimitsValidator::new().validate(&(), &tx).unwrap_err();
        assert!(matches!(
            err,
            TransactionValidationError::ExceedsStealthTransactionLimit { limit: "outputs", .. }
        ));
    }
}
