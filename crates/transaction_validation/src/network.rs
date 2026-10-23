// Copyright 2025 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use log::warn;
use tari_ootle_transaction::{Network, Transaction};

use crate::{TransactionValidationError, Validator};

const LOG_TARGET: &str = "tari::ootle::mempool::validators::network";

#[derive(Debug)]
pub struct TransactionNetworkValidator {
    network: Network,
}

impl TransactionNetworkValidator {
    pub fn new(network: Network) -> Self {
        Self { network }
    }
}

impl Validator<Transaction> for TransactionNetworkValidator {
    type Context = ();
    type Error = TransactionValidationError;

    fn validate(&self, _context: &Self::Context, tx: &Transaction) -> Result<(), Self::Error> {
        let tx_network =
            Network::try_from(tx.network()).map_err(|error| TransactionValidationError::UnknownNetwork {
                byte: tx.network(),
                details: error.to_string(),
            })?;

        if tx_network != self.network {
            warn!(target: LOG_TARGET, "TransactionNetworkValidator - FAIL: mismatching networks: TX: {} != Current: {}", tx_network, self.network);
            return Err(Self::Error::NetworkMismatch {
                actual: tx_network,
                expected: self.network,
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
        TransactionSignature,
        UnsealedTransactionV1,
        UnsignedTransactionV1,
    };
    use tari_template_lib::types::crypto::RistrettoPublicKeyBytes;

    use crate::{TransactionNetworkValidator, TransactionValidationError, Validator};

    fn tx(network_byte: u8) -> Transaction {
        Transaction::new(
            UnsealedTransactionV1::new(
                UnsignedTransactionV1::new(network_byte, vec![], vec![], IndexSet::new(), None, Epoch(1), false),
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
    fn unknown_network() {
        let network_byte = 9u8;
        let validator = TransactionNetworkValidator::new(Network::LocalNet);
        let tx = tx(network_byte);
        let result = validator.validate(&(), &tx);
        assert!(result.is_err());
        assert!(matches!(
            result.err().unwrap(),
            TransactionValidationError::UnknownNetwork { .. }
        ));
    }

    #[test]
    fn network_mismatch() {
        let network_byte = Network::MainNet.as_byte();
        let validator = TransactionNetworkValidator::new(Network::LocalNet);
        let tx = tx(network_byte);
        let result = validator.validate(&(), &tx);
        assert!(result.is_err());
        assert!(matches!(
            result.err().unwrap(),
            TransactionValidationError::NetworkMismatch { actual: _, expected: _ },
        ));
    }

    #[test]
    fn network_ok() {
        let network_byte = Network::LocalNet.as_byte();
        let validator = TransactionNetworkValidator::new(Network::LocalNet);
        let tx = tx(network_byte);
        let result = validator.validate(&(), &tx);
        assert!(result.is_ok());
    }
}
