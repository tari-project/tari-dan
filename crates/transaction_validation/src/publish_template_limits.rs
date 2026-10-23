// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use log::warn;
use tari_engine_types::limits::{ENGINE_LIMITS, MAX_PUBLISH_TEMPLATES_PER_TRANSACTION};
use tari_ootle_transaction::{Instruction, Transaction};

use crate::{TransactionValidationError, Validator};

const LOG_TARGET: &str = "tari::ootle::mempool::validators::publish_template_limits";

/// Rejects transactions that break one of the engine's publish rules: more than
/// [`MAX_PUBLISH_TEMPLATES_PER_TRANSACTION`] `PublishTemplate` instructions, a template published from the fee
/// instructions, or a binary larger than `ENGINE_LIMITS.max_template_binary_size_bytes`.
///
/// A publish compiles the binary, which costs two orders of magnitude more than the compute credit a fee intent runs
/// on, and it is charged only once the fee intent has been paid for. Fee instructions exist to source the fee, and no
/// way of sourcing a fee involves publishing a template.
///
/// Each is a pure function of the transaction, so all three are decided here rather than during execution. This
/// validator backs both mempool ingress and block validation, so every validator applies the rules to every
/// transaction before it can execute: a transaction breaking one is never gossiped or stored, and a block carrying
/// one is rejected.
#[derive(Debug, Clone, Default)]
pub struct PublishTemplateLimitValidator;

impl PublishTemplateLimitValidator {
    pub fn new() -> Self {
        Self
    }
}

impl Validator<Transaction> for PublishTemplateLimitValidator {
    type Context = ();
    type Error = TransactionValidationError;

    fn validate(&self, _context: &(), transaction: &Transaction) -> Result<(), Self::Error> {
        if transaction
            .fee_instructions()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::PublishTemplate { .. }))
        {
            let transaction_id = transaction.calculate_id();
            warn!(
                target: LOG_TARGET,
                "PublishTemplateLimitValidator - FAIL: {transaction_id} publishes a template in its fee instructions"
            );
            return Err(TransactionValidationError::PublishTemplateInFeeInstructions { transaction_id });
        }

        // Count across both instruction lists, matching `Transaction::has_publish_template`.
        let count = transaction
            .instructions()
            .iter()
            .chain(transaction.fee_instructions())
            .filter(|instruction| matches!(instruction, Instruction::PublishTemplate { .. }))
            .count();

        // The binary is charged for the Cranelift compile it makes every validator run, and that charge is what
        // sets the size bound, so a binary past it can never be published however much fee it carries.
        //
        // `publish_templates_iter` covers both instruction lists, so this holds independently of the fee-instruction
        // rule above. A publish whose blob index does not resolve contributes no size and passes here;
        // `validate_blob_references` is what rejects that, and the engine re-checks the size at execution either way.
        let max_binary_size = ENGINE_LIMITS.max_template_binary_size_bytes;
        if let Some(size) = transaction
            .publish_templates_iter()
            .map(<[u8]>::len)
            .find(|size| *size > max_binary_size)
        {
            let transaction_id = transaction.calculate_id();
            warn!(
                target: LOG_TARGET,
                "PublishTemplateLimitValidator - FAIL: {transaction_id} publishes a {size}-byte binary, maximum is \
                 {max_binary_size}"
            );
            return Err(TransactionValidationError::PublishTemplateBinaryTooLarge {
                transaction_id,
                max: max_binary_size,
                actual: size,
            });
        }

        if count > MAX_PUBLISH_TEMPLATES_PER_TRANSACTION {
            let transaction_id = transaction.calculate_id();
            warn!(
                target: LOG_TARGET,
                "PublishTemplateLimitValidator - FAIL: {transaction_id} has {count} publish-template instructions, \
                 maximum is {MAX_PUBLISH_TEMPLATES_PER_TRANSACTION}"
            );
            return Err(TransactionValidationError::TooManyPublishTemplateInstructions {
                transaction_id,
                max: MAX_PUBLISH_TEMPLATES_PER_TRANSACTION,
                actual: count,
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
        Sr25519SignatureBytes,
        TransactionSealSignature,
        TransactionSignature,
        UnsealedTransactionV1,
        UnsignedTransactionV1,
    };
    use tari_template_lib::types::crypto::RistrettoPublicKeyBytes;

    use super::*;

    fn publish_template() -> Instruction {
        Instruction::PublishTemplate {
            binary: 0,
            metadata_hash: None,
        }
    }

    fn tx_with_instructions(instructions: Vec<Instruction>) -> Transaction {
        tx(vec![], instructions)
    }

    fn tx(fee_instructions: Vec<Instruction>, instructions: Vec<Instruction>) -> Transaction {
        Transaction::new(
            UnsealedTransactionV1::new(
                UnsignedTransactionV1::new(
                    Network::LocalNet.as_byte(),
                    fee_instructions,
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
    fn accepts_no_publish_template() {
        let tx = tx_with_instructions(vec![]);
        PublishTemplateLimitValidator::new().validate(&(), &tx).unwrap();
    }

    #[test]
    fn accepts_publish_templates_at_the_limit() {
        let tx = tx_with_instructions(
            (0..MAX_PUBLISH_TEMPLATES_PER_TRANSACTION)
                .map(|_| publish_template())
                .collect(),
        );
        PublishTemplateLimitValidator::new().validate(&(), &tx).unwrap();
    }

    /// A binary past the cap can never succeed however much fee it carries, and the transaction byte
    /// cap sits above it, so nothing else at ingress stops it being gossiped and stored first.
    #[test]
    fn rejects_a_binary_over_the_publish_cap() {
        let over = ENGINE_LIMITS.max_template_binary_size_bytes + 1;
        let tx = Transaction::builder_localnet(Epoch(1))
            .publish_template(vec![0u8; over])
            .build_and_seal(&Default::default());

        let err = PublishTemplateLimitValidator::new().validate(&(), &tx).unwrap_err();
        assert!(matches!(
            err,
            TransactionValidationError::PublishTemplateBinaryTooLarge { max, actual, .. }
            if max == ENGINE_LIMITS.max_template_binary_size_bytes && actual == over
        ));
    }

    #[test]
    fn accepts_a_binary_at_the_publish_cap() {
        let tx = Transaction::builder_localnet(Epoch(1))
            .publish_template(vec![0u8; ENGINE_LIMITS.max_template_binary_size_bytes])
            .build_and_seal(&Default::default());

        PublishTemplateLimitValidator::new().validate(&(), &tx).unwrap();
    }

    #[test]
    fn rejects_a_publish_in_the_fee_instructions() {
        let tx = tx(vec![publish_template()], vec![]);
        let err = PublishTemplateLimitValidator::new().validate(&(), &tx).unwrap_err();
        assert!(matches!(
            err,
            TransactionValidationError::PublishTemplateInFeeInstructions { .. }
        ));
    }

    #[test]
    fn rejects_multiple_publish_templates() {
        let over_limit = MAX_PUBLISH_TEMPLATES_PER_TRANSACTION + 1;
        let tx = tx_with_instructions((0..over_limit).map(|_| publish_template()).collect());
        let err = PublishTemplateLimitValidator::new().validate(&(), &tx).unwrap_err();
        assert!(matches!(
            err,
            TransactionValidationError::TooManyPublishTemplateInstructions { max, actual, .. }
            if max == MAX_PUBLISH_TEMPLATES_PER_TRANSACTION && actual == over_limit
        ));
    }
}
