//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use ootle_byte_type::ToByteType;
use rand::{CryptoRng, Rng};
use signature::{hazmat::PrehashSigner, Keypair};
use tari_crypto::{
    keys::{PublicKey, SecretKey},
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_ootle_address::OotleAddress;
use tari_ootle_common_types::{Network, signature::Sr25519SignatureBytes};
use tari_ootle_transaction::{
    Signable,
    TransactionSealSignature,
    TransactionSignature,
    UnsealedTransactionV1,
    UnsignedTransaction,
};
use tari_ootle_wallet_crypto::kdfs;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use crate::{
    key_provider::DiffieHellmanKdfKeyProvider,
    keys::OotleSecretKey,
    signer,
    signer::local_signer::LocalSigner,
    transaction::TransactionSigner,
    wallet::TransactionAuthorization,
    Address,
};

pub type PrivateKeySigner = LocalSigner<OotleSecretKey>;

impl LocalSigner<OotleSecretKey> {
    pub fn new(network: Network, secret: OotleSecretKey) -> Self {
        let address = secret.to_address(network);
        Self {
            address,
            credentials: secret,
        }
    }

    /// Generate a new PrivateKeySigner with a (non-recoverable) random private key.
    pub fn random(network: Network) -> Self {
        Self::random_with(network, &mut rand::rng())
    }

    pub fn random_with<R: Rng + CryptoRng>(network: Network, rng: &mut R) -> Self {
        let secret = OotleSecretKey::random_with(rng);
        Self::new(network, secret)
    }
}

#[async_trait]
impl<C> TransactionSigner for LocalSigner<C>
where C: PrehashSigner<(Sr25519SignatureBytes, RistrettoPublicKey)> + Send + Sync
{
    fn address(&self) -> &Address {
        &self.address
    }

    async fn sign_transaction(&self, message: &UnsealedTransactionV1) -> signer::Result<TransactionSealSignature> {
        let message = message.to_signing_message(());
        let (signature, public_key) = self.credentials.sign_prehash(&message)?;
        let sig = TransactionSealSignature::new(public_key.to_byte_type(), signature);
        Ok(sig)
    }

    async fn sign_authorization(
        &self,
        seal_signer: &RistrettoPublicKeyBytes,
        tx: &UnsignedTransaction,
    ) -> signer::Result<TransactionAuthorization> {
        let message = tx.to_signing_message(seal_signer);
        let (signature, public_key) = self.credentials.sign_prehash(&message)?;
        let sig = TransactionSignature::new(public_key.to_byte_type(), signature);
        Ok(sig.into())
    }
}

#[async_trait]
impl DiffieHellmanKdfKeyProvider for LocalSigner<OotleSecretKey> {
    type Error = ();
    type Key = RistrettoSecretKey;

    async fn create_kdf_dh_key(&self, public_key: &RistrettoPublicKey) -> Result<Self::Key, Self::Error> {
        Ok(kdfs::encrypted_data_dh_kdf_aead(
            self.credentials.view_only_secret(),
            public_key,
        ))
    }
}
