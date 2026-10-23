//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use ootle_byte_type::{FromByteType, ToByteType};
use signature::{Keypair, hazmat::PrehashSigner};
use tari_crypto::{
    keys::{PublicKey, SecretKey},
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_ootle_address::{Network, OotleAddress};
use tari_ootle_common_types::{
    base_layer_hashing::encrypted_data_hasher,
    engine_types::crypto::OutputBody,
    signature::{Sr25519SignatureBytes, TRANSACTION_SIGNING_CONTEXT},
};
use tari_ootle_wallet_crypto::{
    DecryptedData,
    StealthCryptoApi,
    balance_proof::generate_stealth_balance_proof_signature,
    encrypted_data,
    kdfs,
};
use tari_template_lib_types::{
    Amount,
    EncryptedData,
    crypto::PedersenCommitmentBytes,
    stealth::{StealthInput, StealthInputsStatement, StealthTransferStatement},
};

use crate::{
    key_provider,
    key_provider::{DiffieHellmanKdfKeyProvider, LocalKeyProvider, OutputMaskProvider},
    signer,
    signer::StealthKeyPrehashSigner,
    stealth::{
        BurnClaimKeyProvider,
        BurnClaimStatementSpec,
        InputDecryptor,
        StealthOutputStatementFactory,
        StealthProviderError,
        StealthResult,
    },
};

#[derive(Clone)]
pub struct OotleSecretKey {
    network: Network,
    account_secret: RistrettoSecretKey,
    view_only_secret: RistrettoSecretKey,
}

impl OotleSecretKey {
    /// Create an OotleSecretKey from existing raw Ristretto secret keys.
    pub fn new(network: Network, account_secret: RistrettoSecretKey, view_only_secret: RistrettoSecretKey) -> Self {
        Self {
            network,
            account_secret,
            view_only_secret,
        }
    }

    pub fn random(network: Network) -> Self {
        let mut rng = rand::rng();
        Self::random_with(&mut rng, network)
    }

    pub fn random_with<R: rand::Rng + rand::CryptoRng>(rng: &mut R, network: Network) -> Self {
        let account_secret = RistrettoSecretKey::random(rng);
        let view_only_secret = RistrettoSecretKey::random(rng);
        Self {
            network,
            account_secret,
            view_only_secret,
        }
    }

    pub fn account_secret(&self) -> &RistrettoSecretKey {
        &self.account_secret
    }

    pub fn network(&self) -> Network {
        self.network
    }

    pub fn to_address(&self) -> OotleAddress {
        let account_pk = RistrettoPublicKey::from_secret_key(&self.account_secret);
        let view_only_pk = RistrettoPublicKey::from_secret_key(&self.view_only_secret);
        OotleAddress::new(self.network, view_only_pk.to_byte_type(), account_pk.to_byte_type())
    }

    pub fn view_only_secret(&self) -> &RistrettoSecretKey {
        &self.view_only_secret
    }
}

impl PrehashSigner<(Sr25519SignatureBytes, RistrettoPublicKey)> for OotleSecretKey {
    fn sign_prehash(&self, prehash: &[u8]) -> signature::Result<(Sr25519SignatureBytes, RistrettoPublicKey)> {
        let signature = TRANSACTION_SIGNING_CONTEXT.sign(&self.account_secret, prehash);
        let public_key = self.verifying_key();
        Ok((signature, public_key))
    }
}

impl StealthKeyPrehashSigner<(Sr25519SignatureBytes, RistrettoPublicKey)> for OotleSecretKey {
    async fn sign_prehash_with_stealth_key(
        &self,
        public_nonce: &RistrettoPublicKey,
        prehash: &[u8],
    ) -> signer::Result<(Sr25519SignatureBytes, RistrettoPublicKey)> {
        let secret = kdfs::owner_stealth_dh_secret(
            self.network(),
            self.account_secret(),
            self.view_only_secret(),
            public_nonce,
        );
        let signature = TRANSACTION_SIGNING_CONTEXT.sign(&secret, prehash);
        let public_key = RistrettoPublicKey::from_secret_key(&secret);
        Ok((signature, public_key))
    }

    async fn stealth_public_key(&self, public_nonce: &RistrettoPublicKey) -> signer::Result<RistrettoPublicKey> {
        let secret = kdfs::owner_stealth_dh_secret(
            self.network(),
            self.account_secret(),
            self.view_only_secret(),
            public_nonce,
        );
        Ok(RistrettoPublicKey::from_secret_key(&secret))
    }
}

#[async_trait]
impl DiffieHellmanKdfKeyProvider for LocalKeyProvider<OotleSecretKey> {
    async fn create_kdf_dh_encrypted_data_key(
        &self,
        public_key: &RistrettoPublicKey,
    ) -> key_provider::Result<RistrettoSecretKey> {
        Ok(kdfs::dh_kdf_aead(
            encrypted_data_hasher(),
            self.credentials().view_only_secret(),
            public_key,
        ))
    }
}

#[async_trait]
impl InputDecryptor for LocalKeyProvider<OotleSecretKey>
where Self: DiffieHellmanKdfKeyProvider
{
    async fn decrypt_input_data(
        &self,
        commitment: &PedersenCommitmentBytes,
        input: &OutputBody,
        skip_memo: bool,
    ) -> StealthResult<DecryptedData> {
        let sender_nonce_pk =
            input
                .public_nonce
                .try_from_byte_type()
                .map_err(|e| StealthProviderError::DecryptionFailed {
                    commitment: *commitment,
                    details: format!(
                        "Malformed public nonce in output ({e}). This should not happen because this is verified by \
                         the validators."
                    ),
                })?;

        let encryption_key = self
            .create_kdf_dh_encrypted_data_key(&sender_nonce_pk)
            .await
            .map_err(|e| StealthProviderError::DecryptionFailed {
                commitment: *commitment,
                details: format!("Failed to derive encryption key: {}", e),
            })?;

        let decrypted = encrypted_data::unblind_output(commitment, &input.encrypted_data, &encryption_key, skip_memo)
            .map_err(|e| StealthProviderError::DecryptionFailed {
            commitment: *commitment,
            details: e.to_string(),
        })?;

        Ok(decrypted)
    }
}

#[async_trait]
impl BurnClaimKeyProvider for LocalKeyProvider<OotleSecretKey> {
    async fn derive_burn_claim_secret(
        &self,
        sender_offset_public_key: &RistrettoPublicKey,
    ) -> StealthResult<RistrettoSecretKey> {
        Ok(StealthCryptoApi::new()
            .derive_burn_claim_stealth_secret(self.credentials().account_secret(), sender_offset_public_key))
    }

    async fn decrypt_burn_claim_output(
        &self,
        encrypted_data: &EncryptedData,
        commitment: &PedersenCommitmentBytes,
        sender_offset_public_key: &RistrettoPublicKey,
    ) -> StealthResult<DecryptedData> {
        // The L1 burn output's value/mask are bound to the account secret (not the view-only key), so we
        // decrypt with the account secret. `skip_memo` because the L1 output carries no memo we need here.
        let decrypted = StealthCryptoApi::new().decrypt_utxo_data(
            encrypted_data,
            commitment,
            self.credentials().account_secret(),
            sender_offset_public_key,
            true,
        )?;
        Ok(decrypted)
    }

    async fn create_burn_claim_statement(
        &self,
        spec: BurnClaimStatementSpec,
    ) -> StealthResult<StealthTransferStatement> {
        let BurnClaimStatementSpec {
            commitment,
            encrypted_data,
            sender_offset_public_key,
            output,
            revealed_output,
        } = spec;

        let decrypted = self
            .decrypt_burn_claim_output(&encrypted_data, &commitment, &sender_offset_public_key)
            .await?;
        let agg_input_mask = decrypted.mask().clone();

        let (outputs_statement, agg_output_mask) =
            self.generate_outputs_statement(vec![output], revealed_output).await?;

        // The single stealth input is the burn UTXO minted by the `claim_burn` instruction, which the
        // instruction itself authorises — hence a bare commitment with no witness.
        let inputs_statement = StealthInputsStatement::new(vec![StealthInput::from(commitment)], Amount::zero());

        // A burn claim spends the minted burn UTXO, which no covenant gates, so there is nothing to claim. The claims
        // are bound by the balance proof, so they must be settled before it is signed.
        let covenant_claims = Vec::new();

        let balance_proof = generate_stealth_balance_proof_signature(
            &agg_input_mask,
            &agg_output_mask,
            &inputs_statement,
            &outputs_statement,
            &covenant_claims,
        );

        Ok(StealthTransferStatement {
            inputs_statement,
            outputs_statement,
            balance_proof: Some(balance_proof),
            covenant_claims,
        })
    }
}

impl Keypair for OotleSecretKey {
    type VerifyingKey = RistrettoPublicKey;

    fn verifying_key(&self) -> Self::VerifyingKey {
        RistrettoPublicKey::from_secret_key(&self.account_secret)
    }
}

#[async_trait]
impl OutputMaskProvider for OotleSecretKey {
    async fn next_mask(&self) -> key_provider::Result<RistrettoSecretKey> {
        // For simplicity, just generate a random mask here. Another implementation may want to derive it in some
        // deterministic way.
        Ok(RistrettoSecretKey::random(&mut rand::rng()))
    }
}
