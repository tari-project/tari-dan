//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use log::warn;
use tari_ootle_common_types::Epoch;
use tari_ootle_transaction::Transaction;

use crate::{TransactionValidationError, Validator};

const LOG_TARGET: &str = "tari::ootle::mempool::validators::epoch_range";

/// Checks a transaction against the epoch window it declares: not before `min_epoch`, not after
/// `max_epoch`.
///
/// Safe to run wherever a transaction is admitted, including the consensus sequencing path. Both
/// rules fail in the permissive direction for a node whose epoch view lags: a lagging node admits
/// what a node ahead of it would refuse, so it never discards a transaction its committee has
/// already sequenced. The complementary ceiling on how far ahead `max_epoch` may sit is deliberately
/// **not** here — see [`TransactionValidityWindowValidator`].
#[derive(Debug, Default)]
pub struct EpochRangeValidator;

impl EpochRangeValidator {
    pub fn new() -> Self {
        Self
    }
}

impl Validator<Transaction> for EpochRangeValidator {
    type Context = Epoch;
    type Error = TransactionValidationError;

    fn validate(&self, &current_epoch: &Epoch, transaction: &Transaction) -> Result<(), TransactionValidationError> {
        if let Some(min_epoch) = transaction.min_epoch() &&
            current_epoch < min_epoch
        {
            warn!(target: LOG_TARGET, "EpochRangeValidator - FAIL: Current epoch {current_epoch} less than minimum epoch {min_epoch}.");
            return Err(TransactionValidationError::CurrentEpochLessThanMinimum {
                current_epoch,
                min_epoch,
            });
        }

        let max_epoch = transaction.max_epoch();
        if current_epoch > max_epoch {
            warn!(target: LOG_TARGET, "EpochRangeValidator - FAIL: Current epoch {current_epoch} greater than maximum epoch {max_epoch}.");
            return Err(TransactionValidationError::CurrentEpochGreaterThanMaximum {
                current_epoch,
                max_epoch,
            });
        }

        Ok(())
    }
}

/// Caps how far ahead of the current epoch a transaction's `max_epoch` may sit, bounding every
/// transaction's lifetime.
///
/// **Admission only.** Unlike [`EpochRangeValidator`] this rule fails in the *strict* direction for
/// a lagging node: a node an epoch behind computes a lower ceiling and refuses a window a node ahead
/// of it accepts. Running it where a transaction can be silently discarded after being sequenced —
/// the consensus new-transaction gate — would let a lagging shard group refuse to admit a
/// transaction another group had already sequenced, stalling it until that group catches up. The
/// binding enforcement therefore happens at execution against the pinned, cross-group-agreed epoch,
/// where every node reaches the same verdict and an out-of-window transaction is sequenced as an
/// abort rather than dropped.
#[derive(Debug)]
pub struct TransactionValidityWindowValidator {
    max_validity_epochs: u64,
}

impl TransactionValidityWindowValidator {
    pub fn new(max_validity_epochs: u64) -> Self {
        Self { max_validity_epochs }
    }
}

impl Validator<Transaction> for TransactionValidityWindowValidator {
    type Context = Epoch;
    type Error = TransactionValidationError;

    fn validate(&self, &current_epoch: &Epoch, transaction: &Transaction) -> Result<(), TransactionValidationError> {
        let max_epoch = transaction.max_epoch();
        // Saturating: an overflowing ceiling admits every representable max_epoch, which is the
        // correct reading of "no epoch is further ahead than the limit allows".
        let latest_permitted = Epoch(current_epoch.as_u64().saturating_add(self.max_validity_epochs));
        if max_epoch > latest_permitted {
            warn!(
                target: LOG_TARGET,
                "TransactionValidityWindowValidator - FAIL: Maximum epoch {max_epoch} is more than {} epochs beyond \
                 current epoch {current_epoch}.",
                self.max_validity_epochs
            );
            return Err(TransactionValidationError::MaxEpochTooFarAhead {
                current_epoch,
                max_epoch,
                max_validity_epochs: self.max_validity_epochs,
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
        Transaction,
        TransactionSealSignature,
        UnsealedTransactionV1,
        UnsignedTransactionV1,
    };
    use tari_template_lib::types::crypto::RistrettoPublicKeyBytes;

    use super::*;

    const MAX_VALIDITY_EPOCHS: u64 = 10;

    fn transaction(min_epoch: Option<Epoch>, max_epoch: Epoch) -> Transaction {
        Transaction::new(
            UnsealedTransactionV1::new(
                UnsignedTransactionV1::new(
                    Network::LocalNet.as_byte(),
                    vec![],
                    vec![],
                    IndexSet::new(),
                    min_epoch,
                    max_epoch,
                    false,
                ),
                vec![],
            )
            .into(),
            TransactionSealSignature::new(RistrettoPublicKeyBytes::zero(), Sr25519SignatureBytes::zero()),
        )
    }

    /// The pair as composed at mempool ingress: window rules plus the admission ceiling.
    fn validate(current_epoch: Epoch, transaction: &Transaction) -> Result<(), TransactionValidationError> {
        EpochRangeValidator::new().validate(&current_epoch, transaction)?;
        TransactionValidityWindowValidator::new(MAX_VALIDITY_EPOCHS).validate(&current_epoch, transaction)
    }

    #[test]
    fn it_accepts_a_transaction_inside_its_window() {
        let tx = transaction(Some(Epoch(5)), Epoch(12));
        validate(Epoch(5), &tx).unwrap();
        validate(Epoch(12), &tx).unwrap();
    }

    #[test]
    fn it_rejects_a_transaction_before_its_min_epoch() {
        let tx = transaction(Some(Epoch(5)), Epoch(12));
        assert!(matches!(
            validate(Epoch(4), &tx),
            Err(TransactionValidationError::CurrentEpochLessThanMinimum { .. })
        ));
    }

    #[test]
    fn it_rejects_an_expired_transaction() {
        let tx = transaction(None, Epoch(12));
        assert!(matches!(
            validate(Epoch(13), &tx),
            Err(TransactionValidationError::CurrentEpochGreaterThanMaximum { .. })
        ));
    }

    #[test]
    fn it_rejects_a_window_beyond_the_ceiling() {
        let tx = transaction(None, Epoch(11));
        assert!(matches!(
            validate(Epoch(0), &tx),
            Err(TransactionValidationError::MaxEpochTooFarAhead { .. })
        ));
    }

    /// The consensus sequencing path must never refuse a window for being too far ahead: a lagging
    /// node would otherwise discard a transaction another shard group has already sequenced.
    #[test]
    fn the_sequencing_rules_ignore_the_ceiling() {
        let tx = transaction(None, Epoch(u64::MAX));
        EpochRangeValidator::new().validate(&Epoch(1), &tx).unwrap();
    }

    #[test]
    fn the_ceiling_is_inclusive() {
        let tx = transaction(None, Epoch(MAX_VALIDITY_EPOCHS));
        validate(Epoch(0), &tx).unwrap();
    }

    #[test]
    fn a_ceiling_that_overflows_admits_any_max_epoch() {
        let tx = transaction(None, Epoch(u64::MAX));
        TransactionValidityWindowValidator::new(u64::MAX)
            .validate(&Epoch(1), &tx)
            .unwrap();
    }
}
