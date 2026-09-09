//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_ootle_common_types::Epoch;
use tari_ootle_transaction::{Network, Transaction};

use crate::{
    BasicValidations,
    BlobReferenceValidator,
    EpochRangeValidator,
    InputSubstateValidator,
    PublishTemplateLimitValidator,
    SignatureLimitValidator,
    StealthTransactionLimitsValidator,
    TransactionNetworkValidator,
    TransactionSignatureValidator,
    TransactionSizeValidator,
    TransactionValidationError,
    TransactionValidityWindowValidator,
    TransactionWeightValidator,
    Validator,
    WithContext,
};

/// Builds the structural (context-free) mempool validations suitable for any transaction entry
/// point: network match, basic well-formedness, the per-transaction byte and weight caps, blob references,
/// input addressability, and signature verification.
///
/// These never depend on lagging runtime state (epoch, template existence), so they cannot
/// false-reject and are safe to run at the indexer before forwarding to validator committees.
///
/// The validator node does not compose this chain. It assembles an equivalent one in
/// `create_node_transaction_validator` so that it can split the checks that back block validation from the
/// ingress-only ones — a rejection rule in block validation is a consensus rule. The two must be kept in step
/// by hand.
pub fn create_structural_transaction_validator(
    network: Network,
    max_transaction_weight: u64,
    max_transaction_size_bytes: usize,
) -> impl Validator<Transaction, Context = (), Error = TransactionValidationError> {
    TransactionNetworkValidator::new(network)
        .and_then(BasicValidations::new())
        // Bytes before weight: the byte cap is what the gossip message limit is derived from, so a
        // transaction failing it could not have been relayed regardless of what it weighs.
        .and_then(TransactionSizeValidator::new(max_transaction_size_bytes))
        .and_then(BlobReferenceValidator::new())
        .and_then(InputSubstateValidator::new())
        .and_then(TransactionWeightValidator::new(max_transaction_weight))
        .and_then(StealthTransactionLimitsValidator::new())
        .and_then(PublishTemplateLimitValidator::new())
        // Bounds the number of signature verifications the next validator performs.
        .and_then(SignatureLimitValidator::new())
        .and_then(TransactionSignatureValidator)
}

/// Builds the validations an indexer runs against a transaction observed on the gossip topic before
/// storing it and propagating it onward: the structural chain plus the two epoch-window rules.
///
/// [`EpochRangeValidator`] refuses a transaction that can no longer be sequenced, which would
/// otherwise be stored only to sit unprunable until the retention window caught up with a `max_epoch`
/// already in the past. [`TransactionValidityWindowValidator`] is what bounds the other direction and
/// is load-bearing for an indexer specifically: an un-receipted row's retention key *is* its
/// `max_epoch`, so a transaction declaring an arbitrarily distant window would be stored in a row no
/// retention window ever reaches.
///
/// Deliberately excludes the checks that depend on this node's view of runtime state — template
/// existence, output substate conflicts. The indexer's view is not authoritative, and refusing on it
/// would count an honest peer's valid transaction against their gossipsub score.
pub fn create_gossip_transaction_validator(
    network: Network,
    max_transaction_weight: u64,
    max_transaction_size_bytes: usize,
    max_validity_epochs: u64,
) -> impl Validator<Transaction, Context = Epoch, Error = TransactionValidationError> {
    WithContext::<Epoch, Transaction, TransactionValidationError>::new()
        .map_context(
            |_| (),
            create_structural_transaction_validator(network, max_transaction_weight, max_transaction_size_bytes),
        )
        .and_then(EpochRangeValidator::new())
        .and_then(TransactionValidityWindowValidator::new(max_validity_epochs))
}

#[cfg(test)]
mod tests {
    use tari_common_types::types::PrivateKey;
    use tari_ootle_common_types::Epoch;
    use tari_template_lib::types::{Amount, ComponentAddress};

    use super::*;

    const MAX_TRANSACTION_WEIGHT: u64 = 100_000;
    const MAX_TRANSACTION_SIZE_BYTES: usize = 1024 * 1024;
    const MAX_VALIDITY_EPOCHS: u64 = 10;

    fn transaction(max_epoch: Epoch) -> Transaction {
        Transaction::builder_localnet(max_epoch)
            .pay_fee_from_component(ComponentAddress::from_array([1u8; 32]), Amount::new(1000))
            .build_and_seal(&PrivateKey::from(1u64))
    }

    fn validate(current_epoch: Epoch, transaction: &Transaction) -> Result<(), TransactionValidationError> {
        create_gossip_transaction_validator(
            Network::LocalNet,
            MAX_TRANSACTION_WEIGHT,
            MAX_TRANSACTION_SIZE_BYTES,
            MAX_VALIDITY_EPOCHS,
        )
        .validate(&current_epoch, transaction)
    }

    #[test]
    fn it_accepts_a_transaction_inside_its_window() {
        validate(Epoch(5), &transaction(Epoch(12))).unwrap();
    }

    #[test]
    fn it_refuses_an_expired_transaction() {
        assert!(matches!(
            validate(Epoch(13), &transaction(Epoch(12))),
            Err(TransactionValidationError::CurrentEpochGreaterThanMaximum { .. })
        ));
    }

    /// An un-receipted transaction is retained against its own `max_epoch`, so a distant window
    /// keeps a record alive long past the point the transaction could have been sequenced. The
    /// ceiling is what closes that, and it is only in effect because this composition includes it —
    /// the structural chain and the range rules both admit the transaction.
    ///
    /// A modest overshoot is the case worth pinning. It is well inside every numeric bound, so
    /// nothing else in the stack objects to it.
    #[test]
    fn it_refuses_a_window_beyond_the_admission_ceiling() {
        assert!(matches!(
            validate(Epoch(1), &transaction(Epoch(1_000_000))),
            Err(TransactionValidationError::MaxEpochTooFarAhead { .. })
        ));
    }

    /// Beyond any honest epoch view the sender is at fault, which is what lets a flood of unbounded
    /// windows count against the peer sending it rather than being absorbed silently.
    #[test]
    fn an_unbounded_window_is_the_senders_fault() {
        let err = validate(Epoch(1), &transaction(Epoch(u64::MAX))).unwrap_err();
        assert!(err.is_sender_fault());
    }
}
