// Copyright 2025 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use log::warn;
use tari_ootle_transaction::Transaction;

use crate::{TransactionValidationError, Validator};

const LOG_TARGET: &str = "tari::ootle::mempool::validators::weight";

/// Rejects transactions whose [`Transaction::calculate_transaction_weight`] exceeds `max_weight`.
///
/// This bounds the size/IO/execution cost of a single transaction at ingress, before it is gossiped,
/// stored and executed, so one transaction cannot carry a disproportionate payload (e.g. multi-MiB
/// inline arguments or a flood of cheap instructions) into the network. `max_weight` comes from
/// `ConsensusConstants::max_transaction_weight` and is set above the heaviest legitimate transaction
/// (a max-size template publish) so honest transactions are never rejected.
#[derive(Debug, Clone)]
pub struct TransactionWeightValidator {
    max_weight: u64,
}

impl TransactionWeightValidator {
    pub fn new(max_weight: u64) -> Self {
        Self { max_weight }
    }
}

impl Validator<Transaction> for TransactionWeightValidator {
    type Context = ();
    type Error = TransactionValidationError;

    fn validate(&self, _context: &(), transaction: &Transaction) -> Result<(), Self::Error> {
        let weight = transaction.calculate_transaction_weight().as_u64();
        if weight > self.max_weight {
            let transaction_id = transaction.calculate_id();
            warn!(
                target: LOG_TARGET,
                "TransactionWeightValidator - FAIL: {transaction_id} weight {weight} exceeds maximum {}",
                self.max_weight
            );
            return Err(TransactionValidationError::TransactionExceedsMaxWeight {
                transaction_id,
                weight,
                max_weight: self.max_weight,
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
        Instruction,
        Network,
        Sr25519SignatureBytes,
        Transaction,
        TransactionSealSignature,
        TransactionSignature,
        UnsealedTransactionV1,
        UnsignedTransactionV1,
        args::InstructionArg,
    };
    use tari_template_lib::types::{FunctionName, TemplateAddress, crypto::RistrettoPublicKeyBytes};

    use super::*;

    fn tx_with_literal_arg(arg_len: usize) -> Transaction {
        let instructions = vec![Instruction::CallFunction {
            address: TemplateAddress::from_array([0; 32]),
            function: FunctionName::try_from("f").unwrap(),
            args: vec![InstructionArg::raw_literal_bytes(vec![0u8; arg_len])],
        }];
        Transaction::new(
            UnsealedTransactionV1::new(
                UnsignedTransactionV1::new(
                    Network::LocalNet.as_byte(),
                    vec![],
                    instructions,
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
    fn accepts_transaction_within_weight_limit() {
        let validator = TransactionWeightValidator::new(1000);
        // A small literal arg keeps the weight well under the limit.
        let tx = tx_with_literal_arg(30);
        assert!(validator.validate(&(), &tx).is_ok());
    }

    #[test]
    fn rejects_transaction_exceeding_weight_limit() {
        let validator = TransactionWeightValidator::new(1000);
        // A large inline literal argument pushes the weight past the limit.
        let tx = tx_with_literal_arg(6000);
        let err = validator.validate(&(), &tx).unwrap_err();
        assert!(matches!(err, TransactionValidationError::TransactionExceedsMaxWeight {
            max_weight: 1000,
            ..
        }));
    }
}
