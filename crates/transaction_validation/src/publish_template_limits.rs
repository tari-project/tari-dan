// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use log::warn;
use tari_engine_types::limits::MAX_PUBLISH_TEMPLATES_PER_TRANSACTION;
use tari_ootle_transaction::{Instruction, Transaction};

use crate::{TransactionValidationError, Validator};

const LOG_TARGET: &str = "tari::ootle::mempool::validators::publish_template_limits";

/// Rejects transactions carrying more than [`MAX_PUBLISH_TEMPLATES_PER_TRANSACTION`] `PublishTemplate` instructions,
/// or publishing a template from their fee instructions.
///
/// A publish compiles the binary, which costs two orders of magnitude more than the compute credit a fee intent runs
/// on, and it is charged only once the fee intent has been paid for. Fee instructions exist to source the fee, and no
/// way of sourcing a fee involves publishing a template.
///
/// This mirrors the engine's execution-time cap at ingress, rejecting such transactions before they are gossiped,
/// stored and executed. The engine remains the consensus authority; see
/// [`tari_engine_types::limits::MAX_PUBLISH_TEMPLATES_PER_TRANSACTION`].
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
        TransactionSealSignature,
        TransactionSignature,
        UnsealedTransactionV1,
        UnsignedTransactionV1,
    };
    use tari_template_lib::types::crypto::{RistrettoPublicKeyBytes, SchnorrSignatureBytes};

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
                    SchnorrSignatureBytes::zero(),
                )],
            )
            .into(),
            TransactionSealSignature::new(RistrettoPublicKeyBytes::zero(), SchnorrSignatureBytes::zero()),
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
