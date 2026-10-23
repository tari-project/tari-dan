//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ledger_transport::{APDUCommand, APDUErrorCode, Exchange};
use ootle_ledger_common::{
    Instruction,
    OotleStatusWord,
    arg_types::{
        FrameKind,
        GetPublicKeyRequest,
        GetPublicKeyResponse,
        KeyType,
        SEGMENT_LAST_CHUNK,
        SignMode,
        SignTransactionHeader,
        SignTransactionResponse,
        SigningField,
        StealthTweak,
    },
};
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use crate::{decode::DecodeAnswer, error::LedgerClientError};

/// Result of a [`LedgerClient`] operation; `E` is the transport's error type.
pub type LedgerClientResult<T, E> = Result<T, LedgerClientError<E>>;

const CLA: u8 = 0x80;

/// Max APDU data payload per chunk. The APDU data field is at most 255 bytes; stay under it.
const MAX_CHUNK: usize = 250;

/// One field of the canonical signing preimage to stream: its wire tag and borsh bytes.
pub type SegmentRef<'a> = (SigningField, &'a [u8]);

/// Client for the Ootle Ledger app, generic over the APDU transport `T`.
pub struct LedgerClient<T> {
    inner: T,
}

impl<T: Exchange> LedgerClient<T> {
    /// Wrap an APDU transport. The transport is assumed to be connected to a device (or emulator)
    /// with the Ootle app open.
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    /// Return the running app's version string, e.g. `"0.1.0"`.
    pub async fn get_app_version(&self) -> LedgerClientResult<String, T::Error> {
        let command = APDUCommand {
            cla: CLA,
            ins: Instruction::GetVersion as u8,
            p1: 0,
            p2: 0,
            data: Vec::<u8>::new(),
        };
        let answer = self.inner.exchange(&command).await?;
        apdu_err(answer.error_code())?;
        answer.decode()
    }

    /// Return the running app's name, e.g. `"Tari Ootle"`.
    pub async fn get_app_name(&self) -> LedgerClientResult<String, T::Error> {
        let command = APDUCommand {
            cla: CLA,
            ins: Instruction::GetAppName as u8,
            p1: 0,
            p2: 0,
            data: Vec::<u8>::new(),
        };
        let answer = self.inner.exchange(&command).await?;
        apdu_err(answer.error_code())?;
        answer.decode()
    }

    /// Derive a key on-device from the `(account, index, key_type)` BIP-32 path parameters and
    /// return its public key. The secret never leaves the device.
    ///
    /// `stealth` is `Some` to obtain the stealth one-time key for a confidential transfer: the
    /// returned key is `(c + k)·G`, the same key the device signs with when given the tweak's
    /// public nonce in [`Self::sign_transaction`].
    pub async fn get_public_key(
        &self,
        account: u64,
        index: u64,
        key_type: KeyType,
        stealth: Option<StealthTweak>,
    ) -> LedgerClientResult<RistrettoPublicKeyBytes, T::Error> {
        let req = GetPublicKeyRequest {
            account,
            index,
            key_type,
            stealth,
        };

        let command = APDUCommand {
            cla: CLA,
            ins: Instruction::GetPublicKey as u8,
            p1: 0,
            p2: 0,
            data: borsh::to_vec(&req).unwrap(),
        };
        let answer = self.inner.exchange(&command).await?;
        apdu_err(answer.error_code())?;
        let GetPublicKeyResponse { public_key } = answer.decode()?;
        Ok(public_key.into())
    }

    /// Stream a transaction to the device for signing and return the signature once the user
    /// approves on-device.
    ///
    /// `mode` selects the procedure (authorization vs seal). `segments` are the canonical preimage
    /// fields in chain order — see `tari_ootle_transaction::TransactionSignature::signing_preimage_v1`
    /// / `TransactionSealSignature::signing_preimage_v1`. The device recomputes the signing message
    /// from these bytes itself; nothing here is trusted as a precomputed hash.
    ///
    /// `stealth_public_nonce` is `Some` for a confidential (stealth) transfer: the spent UTXO's
    /// sender public nonce `R`, which the device uses to sign with the stealth key `c + k` rather
    /// than the raw account key. It is not part of the signed message.
    pub async fn sign_transaction(
        &self,
        account: u64,
        index: u64,
        key_type: KeyType,
        mode: SignMode,
        stealth_public_nonce: Option<[u8; 32]>,
        segments: &[SegmentRef<'_>],
    ) -> LedgerClientResult<SignTransactionResponse, T::Error> {
        let header = SignTransactionHeader {
            account,
            index,
            key_type,
            mode,
            stealth_public_nonce,
        };
        let header_bytes =
            borsh::to_vec(&header).map_err(|e| LedgerClientError::InvalidResponse { details: e.to_string() })?;
        self.send_sign_chunk(0, FrameKind::Header as u8, header_bytes).await?;

        for (field, bytes) in segments {
            let tag = *field as u8;
            if bytes.is_empty() {
                self.send_sign_chunk(tag | SEGMENT_LAST_CHUNK, FrameKind::Segment as u8, Vec::new())
                    .await?;
                continue;
            }
            let mut chunks = bytes.chunks(MAX_CHUNK).peekable();
            while let Some(chunk) = chunks.next() {
                let p1 = if chunks.peek().is_none() {
                    tag | SEGMENT_LAST_CHUNK
                } else {
                    tag
                };
                self.send_sign_chunk(p1, FrameKind::Segment as u8, chunk.to_vec())
                    .await?;
            }
        }

        let command = APDUCommand {
            cla: CLA,
            ins: Instruction::SignTransaction as u8,
            p1: 0,
            p2: FrameKind::Finalize as u8,
            data: Vec::<u8>::new(),
        };
        let answer = self.inner.exchange(&command).await?;
        apdu_err(answer.error_code())?;
        let response: SignTransactionResponse = answer.decode()?;
        Ok(response)
    }

    /// Send one streaming chunk and assert the device acknowledged it (empty OK expected).
    async fn send_sign_chunk(&self, p1: u8, p2: u8, data: Vec<u8>) -> LedgerClientResult<(), T::Error> {
        let command = APDUCommand {
            cla: CLA,
            ins: Instruction::SignTransaction as u8,
            p1,
            p2,
            data,
        };
        let answer = self.inner.exchange(&command).await?;
        apdu_err(answer.error_code())?;
        Ok(())
    }
}

fn apdu_err<E>(code: Result<APDUErrorCode, u16>) -> LedgerClientResult<(), E> {
    match code {
        Ok(APDUErrorCode::NoError) => Ok(()),
        Ok(code) => Err(LedgerClientError::APDUError { code }),
        Err(code) => OotleStatusWord::try_from(code)
            .map_err(|_| LedgerClientError::APDUOtherCodeError { code })
            .and_then(|app_sw| Err(LedgerClientError::AppError { code: app_sw })),
    }
}

#[cfg(all(test, feature = "speculos-transport"))]
mod tests {
    use super::*;
    use crate::speculos_transport::SpeculosTransport;

    /// Build a Speculos transport, with the base URL overridable via `SPECULOS_URL` (default
    /// `http://localhost:5000`).
    fn speculos_transport() -> SpeculosTransport {
        match std::env::var("SPECULOS_URL") {
            Ok(base) => SpeculosTransport::with_base_url(&base),
            Err(_) => SpeculosTransport::new(),
        }
    }

    #[tokio::test]
    #[ignore = "Requires Speculos to be running with the ootle ledger app."]
    async fn basic_instructions() {
        let client = LedgerClient::new(speculos_transport());

        let version = client.get_app_version().await.unwrap();
        assert_eq!(version, "0.3.0");

        let app_name = client.get_app_name().await.unwrap();
        assert_eq!(app_name, "Tari Ootle");

        let public_key = client.get_public_key(0, 0, KeyType::Transaction, None).await.unwrap();
        assert_eq!(public_key.len(), 32);
        let other_pk = client.get_public_key(0, 0, KeyType::Transaction, None).await.unwrap();
        assert_eq!(public_key, other_pk);
        let other_pk = client.get_public_key(0, 1, KeyType::Transaction, None).await.unwrap();
        assert_ne!(public_key, other_pk);
        let other_pk = client.get_public_key(0, 0, KeyType::Account, None).await.unwrap();
        assert_ne!(public_key, other_pk);
        let other_pk = client.get_public_key(1, 0, KeyType::Transaction, None).await.unwrap();
        assert_ne!(public_key, other_pk);
    }

    use indexmap::IndexSet;
    use tari_ootle_common_types::signature::Sr25519SignatureBytes;
    use tari_ootle_transaction::{
        Blob,
        Blobs,
        Epoch,
        Instruction,
        MainSigner,
        PreimageSegment,
        TransactionSealSignature,
        TransactionSignature,
        UnsealedTransactionV1,
        UnsignedTransactionV1,
    };

    fn sample_unsigned() -> UnsignedTransactionV1 {
        let mut blobs = Blobs::empty();
        blobs.push(Blob::from(vec![1u8, 2, 3])).unwrap();
        UnsignedTransactionV1 {
            network: 1,
            fee_instructions: vec![Instruction::DropAllProofsInWorkspace],
            instructions: vec![
                Instruction::DropAllProofsInWorkspace,
                Instruction::PutLastInstructionOutputOnWorkspace { key: 7 },
            ],
            inputs: IndexSet::new(),
            min_epoch: None,
            max_epoch: Epoch(1),
            main_signer: MainSigner::None,
            dry_run: false,
            nonce: 0,
            blobs,
        }
    }

    fn to_refs(segments: &[PreimageSegment]) -> Vec<SegmentRef<'_>> {
        segments
            .iter()
            // The numeric equivalence of the two field enums is asserted by the
            // `preimage_field_tags_match_protocol` test.
            .map(|seg| (SigningField::try_from(seg.field as u8).unwrap(), seg.bytes.as_slice()))
            .collect()
    }

    #[tokio::test]
    #[ignore = "Requires Speculos to be running with the ootle ledger app."]
    async fn sign_authorization_roundtrip() {
        let client = LedgerClient::new(speculos_transport());
        let (account, index) = (0u64, 0u64);

        // Any public key works as the seal-signer context for an authorization signature.
        let seal_signer = client
            .get_public_key(account, index, KeyType::Account, None)
            .await
            .unwrap();
        let tx = sample_unsigned();

        let segments = TransactionSignature::signing_preimage_v1(&seal_signer, &tx);
        let response = client
            .sign_transaction(
                account,
                index,
                KeyType::Account,
                SignMode::AddSigner,
                None,
                &to_refs(&segments),
            )
            .await
            .unwrap();

        let signature = TransactionSignature::new(
            RistrettoPublicKeyBytes::from(response.public_key),
            Sr25519SignatureBytes::from_array(response.signature),
        );
        assert!(
            signature.verify_v1(&seal_signer, &tx),
            "device authorization signature did not verify"
        );
    }

    #[tokio::test]
    #[ignore = "Requires Speculos to be running with the ootle ledger app."]
    async fn seal_roundtrip() {
        let client = LedgerClient::new(speculos_transport());
        let (account, index) = (0u64, 0u64);

        let unsealed = UnsealedTransactionV1::new(sample_unsigned(), vec![]);
        let segments = TransactionSealSignature::signing_preimage_v1(&unsealed);
        let response = client
            .sign_transaction(
                account,
                index,
                KeyType::Account,
                SignMode::Seal,
                None,
                &to_refs(&segments),
            )
            .await
            .unwrap();

        let seal = TransactionSealSignature::new(
            RistrettoPublicKeyBytes::from(response.public_key),
            Sr25519SignatureBytes::from_array(response.signature),
        );
        assert!(seal.verify_v1(&unsealed), "device seal signature did not verify");
    }

    #[tokio::test]
    #[ignore = "Requires Speculos to be running with the ootle ledger app."]
    async fn sign_stealth_roundtrip() {
        use ootle_network::Network;
        use tari_crypto::{
            keys::{PublicKey, SecretKey},
            ristretto::{RistrettoPublicKey, RistrettoSecretKey},
            tari_utilities::ByteArray,
        };
        use tari_ootle_wallet_crypto::kdfs::owner_stealth_dh_stealth_address;

        let client = LedgerClient::new(speculos_transport());
        let (account, index) = (0u64, 0u64);

        // The device's account public key, and a fresh sender ephemeral nonce (r, R) standing in for
        // the sender nonce of a received stealth UTXO being spent.
        let account_pk_bytes = client
            .get_public_key(account, index, KeyType::Account, None)
            .await
            .unwrap();
        let account_pk = RistrettoPublicKey::from_canonical_bytes(&account_pk_bytes[..]).unwrap();
        // The view key at the same path keys the stealth tweak.
        let view_pk_bytes = client
            .get_public_key(account, index, KeyType::ViewOnlyKey, None)
            .await
            .unwrap();
        let view_pk = RistrettoPublicKey::from_canonical_bytes(&view_pk_bytes[..]).unwrap();
        let r = RistrettoSecretKey::random(&mut rand::rng());
        let r_pub = RistrettoPublicKey::from_secret_key(&r);
        let mut nonce = [0u8; 32];
        nonce.copy_from_slice(r_pub.as_bytes());

        // Any public key works as the seal-signer context for an authorization signature.
        let seal_signer = account_pk_bytes;
        let tx = sample_unsigned();
        let segments = TransactionSignature::signing_preimage_v1(&seal_signer, &tx);

        let response = client
            .sign_transaction(
                account,
                index,
                KeyType::Account,
                SignMode::AddSigner,
                Some(nonce),
                &to_refs(&segments),
            )
            .await
            .unwrap();

        // The device must sign with the stealth key, not the raw account key.
        assert_ne!(
            response.public_key, *account_pk_bytes,
            "device signed with the account key"
        );

        // The returned key must equal the sender-derived stealth address for this network.
        let network = Network::try_from(tx.network).unwrap();
        let expected = owner_stealth_dh_stealth_address(network, &account_pk, &view_pk, &r);
        assert_eq!(
            &response.public_key[..],
            expected.as_bytes(),
            "unexpected stealth address"
        );

        // And the stealth signature verifies as a transaction signature.
        let signature = TransactionSignature::new(
            RistrettoPublicKeyBytes::from(response.public_key),
            Sr25519SignatureBytes::from_array(response.signature),
        );
        assert!(
            signature.verify_v1(&seal_signer, &tx),
            "device stealth signature did not verify"
        );

        // `GetPublicKey` must resolve the very key the device signs with, so callers can bind
        // authorizations to the seal key before any signature is made.
        let queried = client
            .get_public_key(
                account,
                index,
                KeyType::Account,
                Some(StealthTweak {
                    network: tx.network,
                    public_nonce: nonce,
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            *queried, response.public_key,
            "GetPublicKey disagrees with the signing key"
        );
    }

    #[tokio::test]
    #[ignore = "Requires Speculos to be running with the ootle ledger app."]
    async fn stealth_public_key_derivation() {
        use ootle_network::Network;
        use tari_crypto::{
            keys::{PublicKey, SecretKey},
            ristretto::{RistrettoPublicKey, RistrettoSecretKey},
            tari_utilities::ByteArray,
        };

        let client = LedgerClient::new(speculos_transport());
        let (account, index) = (0u64, 0u64);

        let mut rng = rand::rng();
        let nonce_of = |secret: &RistrettoSecretKey| {
            let mut bytes = [0u8; 32];
            bytes.copy_from_slice(RistrettoPublicKey::from_secret_key(secret).as_bytes());
            bytes
        };
        let nonce = nonce_of(&RistrettoSecretKey::random(&mut rng));
        let other_nonce = nonce_of(&RistrettoSecretKey::random(&mut rng));

        let stealth_key = |network: Network, public_nonce: [u8; 32]| {
            client.get_public_key(
                account,
                index,
                KeyType::Account,
                Some(StealthTweak {
                    network: network.as_byte(),
                    public_nonce,
                }),
            )
        };

        let key = stealth_key(Network::LocalNet, nonce).await.unwrap();
        assert_eq!(
            key,
            stealth_key(Network::LocalNet, nonce).await.unwrap(),
            "stealth key is not deterministic"
        );
        assert_ne!(
            key,
            stealth_key(Network::LocalNet, other_nonce).await.unwrap(),
            "stealth key is not bound to the public nonce"
        );
        assert_ne!(
            key,
            stealth_key(Network::Igor, nonce).await.unwrap(),
            "stealth key is not bound to the network"
        );
        assert_ne!(
            key,
            client
                .get_public_key(account, index, KeyType::Account, None)
                .await
                .unwrap(),
            "stealth key equals the untweaked account key"
        );
    }
}
