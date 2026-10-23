//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Ledger hardware-wallet signer.
//!
//! [`LedgerSigner`] implements [`TransactionSigner`], [`TransactionSealSigner`] and
//! [`TransactionStealthKeySigner`] by streaming the canonical signing preimage to an Ootle Ledger
//! device, which recomputes the signing message, displays a summary for the user to approve, and
//! returns a Ristretto Schnorr signature. The two procedures map to the device's two signing modes:
//!   - [`TransactionSigner::sign_authorization`] → [`SignMode::AddSigner`]
//!   - [`TransactionSigner::sign_transaction`] / [`TransactionSealSigner::seal_transaction`] → [`SignMode::Seal`]
//!
//! Transactions are signed with the account key branch ([`KeyType::Account`]), matching the
//! address returned by [`LedgerSigner::address`]. For a confidential (stealth) transfer, the
//! [`TransactionStealthKeySigner`] methods pass the spent UTXO's sender public nonce so the device
//! signs with the stealth-derived key `c + k` instead; the preimage and modes are otherwise
//! identical to the public path.

use async_trait::async_trait;
use ootle_ledger_client::{Exchange, LedgerClient, LedgerClientError};
use ootle_ledger_common::arg_types::{KeyType, SignMode, SigningField, StealthTweak};
use tari_crypto::{ristretto::RistrettoPublicKey, tari_utilities::ByteArray};
use tari_ootle_address::OotleAddress;
use tari_ootle_common_types::signature::Sr25519SignatureBytes;
use tari_ootle_transaction::{
    IntoSigned,
    PreimageField,
    PreimageSegment,
    Transaction,
    TransactionSealSignature,
    TransactionSignature,
    UnsealedTransaction,
    UnsignedTransaction,
};
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use crate::{
    Address,
    Network,
    signer::{self, SignerError},
    transaction::{TransactionSealSigner, TransactionSigner, TransactionStealthKeySigner},
    wallet::TransactionAuthorization,
};

/// The oldest device app whose stealth keys this signer can use. Earlier apps tweak the account key
/// with `H(k·R)`, which matches no output a current sender creates.
const MIN_APP_VERSION: (u64, u64, u64) = (0, 3, 0);

/// A signer backed by an Ootle Ledger device over an arbitrary APDU transport `T`.
pub struct LedgerSigner<T> {
    client: LedgerClient<T>,
    address: Address,
    account: u64,
    index: u64,
}

impl<T> LedgerSigner<T>
where
    T: Exchange,
    T::Error: core::fmt::Display,
{
    /// Connect to a device-backed signer: check the device app is new enough, fetch the account and
    /// view-only public keys for `(account, index)` and derive the wallet [`Address`].
    pub async fn connect(client: LedgerClient<T>, network: Network, account: u64, index: u64) -> signer::Result<Self> {
        let app_version = client.get_app_version().await.map_err(map_client_err)?;
        check_app_version(&app_version)?;
        let account_pk = client
            .get_public_key(account, index, KeyType::Account, None)
            .await
            .map_err(map_client_err)?;
        let view_only_pk = client
            .get_public_key(account, index, KeyType::ViewOnlyKey, None)
            .await
            .map_err(map_client_err)?;
        let address = OotleAddress::new(network, view_only_pk, account_pk);
        Ok(Self {
            client,
            address,
            account,
            index,
        })
    }

    /// Construct directly from an already-known address (skips the device round-trips). The caller
    /// is responsible for ensuring `address` corresponds to `(account, index)` on the device: both its
    /// account key and its view key must be the device's keys at that path, because the device tweaks
    /// stealth spend keys with its own view key. The caller is also responsible for the device running
    /// an app this signer supports (see [`Self::connect`]).
    pub fn with_address(client: LedgerClient<T>, address: Address, account: u64, index: u64) -> Self {
        Self {
            client,
            address,
            account,
            index,
        }
    }

    /// Stream a preimage to the device and parse the returned public key + signature.
    ///
    /// `stealth_public_nonce` is `Some` for a confidential transfer, signing with the stealth key
    /// `c + k` derived from the spent UTXO's sender public nonce.
    async fn stream(
        &self,
        mode: SignMode,
        stealth_public_nonce: Option<[u8; 32]>,
        segments: Vec<PreimageSegment>,
    ) -> signer::Result<(RistrettoPublicKeyBytes, Sr25519SignatureBytes)> {
        let refs: Vec<(SigningField, &[u8])> = segments
            .iter()
            .map(|seg| (to_wire(seg.field), seg.bytes.as_slice()))
            .collect();

        let response = self
            .client
            .sign_transaction(
                self.account,
                self.index,
                KeyType::Account,
                mode,
                stealth_public_nonce,
                &refs,
            )
            .await
            .map_err(map_client_err)?;

        let public_key = RistrettoPublicKeyBytes::from(response.public_key);
        let signature = Sr25519SignatureBytes::from_array(response.signature);
        Ok((public_key, signature))
    }

    /// A stealth key is bound to a network, and the two APDUs take that network from different
    /// places: `GetPublicKey` from this signer's address, signing from the transaction's own
    /// `Network` field. They must agree, or the key resolved up front is not the key the device
    /// signs with and every authorization commits to the wrong seal signer.
    fn check_stealth_network(&self, tx_network: u8) -> signer::Result<()> {
        let network = self.address.network().as_byte();
        if tx_network != network {
            return Err(SignerError::other(format!(
                "cannot sign a transaction for network {tx_network} with a stealth key derived on network {network}",
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl<T> TransactionSigner for LedgerSigner<T>
where
    T: Exchange + Send + Sync,
    T::Error: core::fmt::Display + Send + Sync,
{
    fn address(&self) -> &Address {
        &self.address
    }

    async fn sign_transaction(&self, message: &UnsealedTransaction) -> signer::Result<TransactionSealSignature> {
        let UnsealedTransaction::V1(unsealed) = message;
        let segments = TransactionSealSignature::signing_preimage_v1(unsealed);
        let (public_key, signature) = self.stream(SignMode::Seal, None, segments).await?;
        Ok(TransactionSealSignature::new(public_key, signature))
    }

    async fn sign_authorization(
        &self,
        seal_signer: &RistrettoPublicKeyBytes,
        tx: &UnsignedTransaction,
    ) -> signer::Result<TransactionAuthorization> {
        let UnsignedTransaction::V1(unsigned) = tx;
        let segments = TransactionSignature::signing_preimage_v1(seal_signer, unsigned);
        let (public_key, signature) = self.stream(SignMode::AddSigner, None, segments).await?;
        Ok(TransactionSignature::new(public_key, signature).into())
    }
}

#[async_trait]
impl<T> TransactionStealthKeySigner for LedgerSigner<T>
where
    T: Exchange + Send + Sync,
    T::Error: core::fmt::Display + Send + Sync,
{
    async fn stealth_public_key(&self, public_nonce: &RistrettoPublicKey) -> signer::Result<RistrettoPublicKeyBytes> {
        self.client
            .get_public_key(
                self.account,
                self.index,
                KeyType::Account,
                Some(StealthTweak {
                    network: self.address.network().as_byte(),
                    public_nonce: nonce_bytes(public_nonce),
                }),
            )
            .await
            .map_err(map_client_err)
    }

    async fn sign_authorization_with_stealth(
        &self,
        public_nonce: &RistrettoPublicKey,
        seal_signer: &RistrettoPublicKeyBytes,
        tx: &UnsignedTransaction,
    ) -> signer::Result<TransactionAuthorization> {
        let UnsignedTransaction::V1(unsigned) = tx;
        self.check_stealth_network(unsigned.network)?;
        let segments = TransactionSignature::signing_preimage_v1(seal_signer, unsigned);
        let (public_key, signature) = self
            .stream(SignMode::AddSigner, Some(nonce_bytes(public_nonce)), segments)
            .await?;
        Ok(TransactionSignature::new(public_key, signature).into())
    }

    async fn seal_transaction_with_stealth(
        &self,
        public_nonce: &RistrettoPublicKey,
        message: &UnsealedTransaction,
    ) -> signer::Result<TransactionSealSignature> {
        let UnsealedTransaction::V1(unsealed) = message;
        self.check_stealth_network(unsealed.unsigned_transaction().network)?;
        let segments = TransactionSealSignature::signing_preimage_v1(unsealed);
        let (public_key, signature) = self
            .stream(SignMode::Seal, Some(nonce_bytes(public_nonce)), segments)
            .await?;
        Ok(TransactionSealSignature::new(public_key, signature))
    }
}

#[async_trait]
impl<T> TransactionSealSigner for LedgerSigner<T>
where
    T: Exchange + Send + Sync,
    T::Error: core::fmt::Display + Send + Sync,
{
    async fn seal_transaction(&self, tx: UnsealedTransaction) -> signer::Result<Transaction> {
        let signature = self.sign_transaction(&tx).await?;
        Ok(<UnsealedTransaction as IntoSigned<()>>::into_signed(tx, signature))
    }
}

fn check_app_version(version: &str) -> signer::Result<()> {
    let parsed = version
        .split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()
        .filter(|parts| parts.len() == 3)
        .map(|parts| (parts[0], parts[1], parts[2]));
    match parsed {
        Some(found) if found >= MIN_APP_VERSION => Ok(()),
        Some(_) => {
            let (major, minor, patch) = MIN_APP_VERSION;
            Err(SignerError::other(format!(
                "Ledger app {version} derives stealth keys this wallet cannot use; update the Ootle Ledger app to \
                 {major}.{minor}.{patch} or later"
            )))
        },
        None => Err(SignerError::other(format!(
            "Ledger app reported an unrecognised version {version:?}"
        ))),
    }
}

fn map_client_err<E: core::fmt::Display>(err: LedgerClientError<E>) -> SignerError {
    SignerError::other(err.to_string())
}

/// Compressed 32-byte encoding of a public nonce for the device's stealth-key derivation.
fn nonce_bytes(public_nonce: &RistrettoPublicKey) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(public_nonce.as_bytes());
    bytes
}

/// Map a transaction-crate preimage field to its on-the-wire protocol tag. The numeric equivalence
/// is asserted by `preimage_field_tags_match_protocol` in the ledger client tests.
fn to_wire(field: PreimageField) -> SigningField {
    match field {
        PreimageField::SchemaVersion => SigningField::SchemaVersion,
        PreimageField::SealSigner => SigningField::SealSigner,
        PreimageField::Network => SigningField::Network,
        PreimageField::FeeInstructions => SigningField::FeeInstructions,
        PreimageField::Instructions => SigningField::Instructions,
        PreimageField::Inputs => SigningField::Inputs,
        PreimageField::MinEpoch => SigningField::MinEpoch,
        PreimageField::MaxEpoch => SigningField::MaxEpoch,
        PreimageField::MainSigner => SigningField::MainSigner,
        PreimageField::DryRun => SigningField::DryRun,
        PreimageField::BlobHashes => SigningField::BlobHashes,
        PreimageField::Nonce => SigningField::TransactionNonce,
        PreimageField::Signatures => SigningField::Signatures,
    }
}

#[cfg(test)]
mod tests {
    use ootle_ledger_client::ledger_transport::{APDUAnswer, APDUCommand, async_trait as transport_async_trait};
    use tari_crypto::{
        keys::{PublicKey, SecretKey},
        ristretto::RistrettoSecretKey,
    };
    use tari_ootle_transaction::{Epoch, Transaction as TransactionBuilderEntry, UnsealedTransactionV1};

    use super::*;

    /// A transport that fails the test if the device is contacted: the network guard must reject
    /// before any APDU is sent.
    struct NoDevice;

    #[transport_async_trait]
    impl Exchange for NoDevice {
        type AnswerType = Vec<u8>;
        type Error = std::io::Error;

        async fn exchange<I>(&self, _command: &APDUCommand<I>) -> Result<APDUAnswer<Self::AnswerType>, Self::Error>
        where I: core::ops::Deref<Target = [u8]> + Send + Sync {
            panic!("the device must not be contacted for a transaction on the wrong network");
        }
    }

    fn signer_on(network: Network) -> LedgerSigner<NoDevice> {
        let mut rng = rand::rng();
        let mut key = || {
            RistrettoPublicKeyBytes::from_bytes(
                RistrettoPublicKey::from_secret_key(&RistrettoSecretKey::random(&mut rng)).as_bytes(),
            )
            .unwrap()
        };
        let address = OotleAddress::new(network, key(), key());
        LedgerSigner::with_address(LedgerClient::new(NoDevice), address, 0, 0)
    }

    /// The stealth key `GetPublicKey` returns is derived under the signer's network, while the device
    /// derives the key it signs with from the transaction's own `Network` field. Signing a transaction
    /// from another network would silently sign with a different key than the one the caller bound its
    /// authorizations to.
    #[tokio::test]
    async fn signing_a_transaction_from_another_network_is_rejected() {
        let signer = signer_on(Network::Igor);
        let tx = TransactionBuilderEntry::builder(Network::LocalNet, Epoch(1)).build_unsigned();
        let nonce = RistrettoPublicKey::from_secret_key(&RistrettoSecretKey::random(&mut rand::rng()));

        let err = signer
            .sign_authorization_with_stealth(&nonce, &RistrettoPublicKeyBytes::default(), &tx)
            .await
            .expect_err("a transaction from another network must not be signed");
        assert!(err.to_string().contains("network"), "unexpected error: {err}");

        let unsealed = UnsealedTransaction::V1(UnsealedTransactionV1::new(
            match tx {
                UnsignedTransaction::V1(unsigned) => unsigned,
            },
            vec![],
        ));
        signer
            .seal_transaction_with_stealth(&nonce, &unsealed)
            .await
            .expect_err("a transaction from another network must not be sealed");
    }

    #[test]
    fn app_versions_below_the_minimum_are_rejected() {
        assert!(check_app_version("0.3.0").is_ok());
        assert!(check_app_version("0.3.1").is_ok());
        assert!(check_app_version("1.0.0").is_ok());
        assert!(check_app_version("0.2.9").is_err());
        assert!(check_app_version("0.2.0").is_err());
        assert!(check_app_version("0.3").is_err());
        assert!(check_app_version("not-a-version").is_err());
    }
}
