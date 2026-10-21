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

#[cfg(test)]
mod tests {
    use tari_common_types::types::PrivateKey;
    use tari_crypto::{keys::PublicKey as _, ristretto::RistrettoPublicKey, tari_utilities::ByteArray};
    use tari_ootle_common_types::Epoch;
    use tari_ootle_transaction::TransactionSignature;
    use tari_template_lib::types::crypto::RistrettoPublicKeyBytes;

    use super::*;

    fn public_key(secret: &PrivateKey) -> RistrettoPublicKeyBytes {
        RistrettoPublicKeyBytes::from_bytes(RistrettoPublicKey::from_secret_key(secret).as_bytes()).unwrap()
    }

    /// Seals with `seal_secret`, which the transaction does not authorize, after `signer` signs.
    fn co_signed(main_signer: MainSigner, signer: &PrivateKey) -> Transaction {
        let seal_secret = PrivateKey::from(1u64);
        let unsigned = Transaction::builder_localnet(Epoch(10))
            .with_main_signer(main_signer)
            .build_unsigned();
        let signature = TransactionSignature::sign(signer, &public_key(&seal_secret), &unsigned);
        unsigned.with_signatures(vec![signature]).seal(&seal_secret)
    }

    #[test]
    fn accepts_a_main_signer_that_signed() {
        let signer = PrivateKey::from(2u64);
        let transaction = co_signed(MainSigner::Authorizer(public_key(&signer)), &signer);

        TransactionSignatureValidator.validate(&(), &transaction).unwrap();
    }

    #[test]
    fn rejects_a_main_signer_that_did_not_sign() {
        let absent = public_key(&PrivateKey::from(3u64));
        let transaction = co_signed(MainSigner::Authorizer(absent), &PrivateKey::from(2u64));
        assert!(transaction.verify_all_signatures());

        let err = TransactionSignatureValidator.validate(&(), &transaction).unwrap_err();
        assert!(matches!(
            err,
            TransactionValidationError::MainSignerDidNotSign { public_key, .. } if public_key == absent
        ));
    }
}
