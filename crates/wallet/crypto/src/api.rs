//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use ootle_byte_type::ConvertFromByteType;
use ootle_network::Network;
use tari_crypto::{
    commitment::HomomorphicCommitmentFactory,
    ristretto::{RistrettoPublicKey, RistrettoSchnorr, RistrettoSecretKey, pedersen::PedersenCommitment},
};
use tari_engine_types::crypto::get_commitment_factory;
use tari_ootle_common_types::base_layer_hashing::ownership_proof_hasher64;
use tari_template_lib_types::{
    Amount,
    EncryptedData,
    ResourceAddress,
    confidential::ConfidentialOutputStatement,
    crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes, SchnorrSignatureBytes, UtxoTag},
    stealth::{RevealedOutput, StealthTransferStatement},
};

use crate::{
    DecryptedData,
    OutputWitness,
    StealthInputWitness,
    StealthOutputWitness,
    StealthProofError,
    WalletCryptoError,
    confidential,
    encrypted_data::{encrypt_data, unblind_output},
    kdfs,
    memo::Memo,
    stealth,
};

const LOG_TARGET: &str = "tari::ootle::wallet::sdk::stealth_crypto";

/// The Ootle base chain has no L1 deployment key, so its burn-claim sidechain binding is `None`.
/// The wallet only ever claims to Ootle (it is not sidechain-aware), so this is a constant rather
/// than a threaded parameter. The validator node, which *can* run a keyed sidechain, sources its
/// own id from config instead. See tari-ootle#445.
pub const OOTLE_SIDECHAIN_ID: Option<RistrettoPublicKeyBytes> = None;

#[derive(Debug, Clone, Copy, Default)]
pub struct StealthCryptoApi;

impl StealthCryptoApi {
    pub const fn new() -> Self {
        Self
    }

    pub fn derive_encrypted_data_key(
        &self,
        public_nonce: &RistrettoPublicKey,
        private_key: &RistrettoSecretKey,
    ) -> RistrettoSecretKey {
        kdfs::encrypted_data_dh_kdf_aead(private_key, public_nonce)
    }

    pub fn generate_transfer_statement<'a, A, Inputs, Outputs>(
        &self,
        inputs: Inputs,
        input_revealed_amount: A,
        output_statements: Outputs,
        revealed_output: Option<RevealedOutput>,
    ) -> Result<StealthTransferStatement, StealthCryptoApiError>
    where
        A: Into<Amount>,
        Inputs: IntoIterator<Item = StealthInputWitness>,
        Inputs::IntoIter: ExactSizeIterator,
        Outputs: IntoIterator<Item = &'a StealthOutputWitness> + Clone,
        Outputs::IntoIter: ExactSizeIterator,
    {
        let stmt = stealth::create_transfer_statement(
            inputs,
            input_revealed_amount
                .into()
                .non_negative_checked()
                .ok_or(StealthCryptoApiError::NegativeAmount)?,
            output_statements,
            revealed_output,
        )?;
        Ok(stmt)
    }

    pub fn derive_stealth_output_tag(
        &self,
        network: Network,
        secret: &RistrettoSecretKey,
        public_key: &RistrettoPublicKey,
        resource_address: &ResourceAddress,
    ) -> UtxoTag {
        kdfs::utxo_tag_stealth_dh(network, public_key, secret, resource_address)
    }

    pub fn derive_stealth_owner_public_key(
        &self,
        network: Network,
        account_public_key: &RistrettoPublicKey,
        view_public_key: &RistrettoPublicKey,
        nonce_secret: &RistrettoSecretKey,
    ) -> RistrettoPublicKey {
        kdfs::owner_stealth_dh_stealth_address(network, account_public_key, view_public_key, nonce_secret)
    }

    pub fn derive_stealth_owner_secret(
        &self,
        network: Network,
        account_secret: &RistrettoSecretKey,
        view_secret: &RistrettoSecretKey,
        public_nonce: &RistrettoPublicKey,
    ) -> RistrettoSecretKey {
        kdfs::owner_stealth_dh_secret(network, account_secret, view_secret, public_nonce)
    }

    /// Recipient-side view-only derivation of an output's one-time spend public key. See
    /// [`kdfs::owner_stealth_dh_public_key`].
    pub fn derive_stealth_owner_public_key_from_view_key(
        &self,
        network: Network,
        account_public_key: &RistrettoPublicKey,
        view_secret: &RistrettoSecretKey,
        public_nonce: &RistrettoPublicKey,
    ) -> RistrettoPublicKey {
        kdfs::owner_stealth_dh_public_key(network, account_public_key, view_secret, public_nonce)
    }

    /// Derive `s = H(p·R) + p` using L1's exact `stealth_address` hash domain. Use only on L1
    /// burn-claim paths where the L1 wallet signed the ownership proof against the matching
    /// stealth address `C = s·G`.
    pub fn derive_burn_claim_stealth_secret(
        &self,
        account_secret: &RistrettoSecretKey,
        sender_offset_public_key: &RistrettoPublicKey,
    ) -> RistrettoSecretKey {
        kdfs::burn_claim_stealth_secret(account_secret, sender_offset_public_key)
    }

    pub fn encrypt_value_and_mask(
        &self,
        amount: u64,
        mask: &RistrettoSecretKey,
        public_key: &RistrettoPublicKey,
        secret: &RistrettoSecretKey,
        memo: Option<&Memo>,
    ) -> Result<EncryptedData, StealthCryptoApiError> {
        let encryption_key = kdfs::encrypted_data_dh_kdf_aead(secret, public_key);
        let data = encrypt_data(amount, mask, &encryption_key, memo)?;
        Ok(data)
    }

    pub fn generate_output_proof<A: Into<Amount>>(
        &self,
        statement: &OutputWitness,
        revealed_amount: A,
    ) -> Result<ConfidentialOutputStatement, StealthCryptoApiError> {
        let proof = confidential::create_output_statement(
            Some(statement).filter(|s| s.amount > 0),
            revealed_amount.into(),
            None,
            Amount::zero(),
        )?;
        Ok(proof)
    }

    pub fn decrypt_utxo_data(
        &self,
        output_encrypted_value: &EncryptedData,
        output_commitment: &PedersenCommitmentBytes,
        claim_secret: &RistrettoSecretKey,
        sender_offset_public_key: &RistrettoPublicKey,
        skip_memo: bool,
    ) -> Result<DecryptedData, StealthCryptoApiError> {
        let encryption_key = kdfs::encrypted_data_dh_kdf_aead(claim_secret, sender_offset_public_key);
        let decrypted = unblind_output(output_commitment, output_encrypted_value, &encryption_key, skip_memo)?;
        Ok(decrypted)
    }

    /// `claimant_pk` is the stealth claim public key `C = H(r·P)·G + P` carried in the L1 burn
    /// proof — the key the `ownership_proof` Schnorr signature commits to.
    pub fn validate_burn_claim_ownership_proof(
        &self,
        network: Network,
        ownership_proof: &SchnorrSignatureBytes,
        commitment: &PedersenCommitmentBytes,
        value: u64,
        claimant_pk: &RistrettoPublicKeyBytes,
    ) -> bool {
        // The wallet only ever claims to the Ootle base chain (`OOTLE_SIDECHAIN_ID` = None), so the
        // sidechain binding is constant here. The `Option<&[u8]>` encoding mirrors the L1 signer's
        // `Option<CompressedPublicKey>` borsh, and the authoritative binding check is on the VN
        // (tari-ootle#445).
        // NOTE: .as_bytes() used because the tari_crypto borsh implementations serialize fixed length bytes as variable
        // length bytes of size 32
        let sidechain_id = OOTLE_SIDECHAIN_ID.as_ref().map(|id| id.as_bytes());
        let message = ownership_proof_hasher64(network)
            .chain(&commitment.as_bytes())
            .chain(&claimant_pk.as_bytes())
            .chain(&sidechain_id)
            .finalize();

        let Ok(commitment) = PedersenCommitment::convert_from_byte_type(commitment) else {
            warn!(target: LOG_TARGET, "Claim burn failed - malformed commitment");
            return false;
        };

        let Ok(proof_of_knowledge) = RistrettoSchnorr::convert_from_byte_type(ownership_proof) else {
            warn!(target: LOG_TARGET, "Claim burn failed - malformed proof of knowledge");
            return false;
        };

        let value_commit = get_commitment_factory().commit_value(&RistrettoSecretKey::default(), value);
        // k.G = C - v.H
        let signer_pk = commitment.as_public_key() - value_commit.as_public_key();

        if !proof_of_knowledge.verify(&signer_pk, message) {
            warn!(target: LOG_TARGET, "Claim burn failed - signature verification failed");
            return false;
        }

        true
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StealthCryptoApiError {
    #[error(transparent)]
    WalletCryptoError(#[from] WalletCryptoError),
    #[error("Confidential proof error: {0}")]
    ConfidentialProofError(#[from] StealthProofError),
    #[error("Negative amount")]
    NegativeAmount,
}
