//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use log::warn;
use tari_ootle_transaction::{MainSigner, Transaction};

use crate::{TransactionValidationError, Validator};

const LOG_TARGET: &str = "tari::ootle::mempool::validators::signature";

#[derive(Debug)]
pub struct TransactionSignatureValidator;

impl Validator<Transaction> for TransactionSignatureValidator {
    type Context = ();
    type Error = TransactionValidationError;

    fn validate(&self, _context: &(), transaction: &Transaction) -> Result<(), TransactionValidationError> {
        if !transaction.has_authorizer() {
            warn!(target: LOG_TARGET, "TransactionSignatureValidator - FAIL: No authorizer");
            return Err(TransactionValidationError::TransactionNotSigned {
                transaction_id: transaction.calculate_id(),
            });
        }

        if let MainSigner::Authorizer(public_key) = transaction.declared_main_signer() &&
            transaction.main_signer().is_none()
        {
            warn!(target: LOG_TARGET, "TransactionSignatureValidator - FAIL: Main signer {public_key} did not sign");
            return Err(TransactionValidationError::MainSignerDidNotSign {
                transaction_id: transaction.calculate_id(),
                public_key: *public_key,
            });
        }

        if !transaction.verify_all_signatures() {
            warn!(target: LOG_TARGET, "TransactionSignatureValidator - FAIL: Invalid signature");
            return Err(TransactionValidationError::InvalidSignature);
        }

        Ok(())
    }
}
