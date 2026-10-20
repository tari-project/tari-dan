//    Copyright 2025 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use blake2::Blake2b;
use chacha20poly1305::{
    AeadInPlace,
    KeyInit,
    Tag,
    XChaCha20Poly1305,
    XNonce,
    aead,
    aead::generic_array::GenericArray,
    consts::U32,
};
use digest::FixedOutput;
use ootle_byte_type::ToByteType;
use rand::Rng;
use tari_crypto::{
    commitment::HomomorphicCommitmentFactory,
    hashing::DomainSeparatedHasher,
    ristretto::RistrettoSecretKey,
};
use tari_engine_types::crypto::get_commitment_factory;
use tari_hashing::TransactionSecureNonceKdfDomain;
use tari_template_lib_types::{EncryptedData, crypto::PedersenCommitmentBytes};
use tari_utilities::{ByteArray, safe_array::SafeArray};
use zeroize::{Zeroize, Zeroizing};

use crate::{DecryptedData, MaskAndValue, WalletCryptoError, kdfs::EncryptedDataKey, memo::Memo};

pub fn unblind_output(
    output_commitment: &PedersenCommitmentBytes,
    output_encrypted_value: &EncryptedData,
    encryption_key: &RistrettoSecretKey,
    skip_memo: bool,
) -> Result<DecryptedData, WalletCryptoError> {
    let decrypted = decrypt_data(encryption_key, output_commitment, output_encrypted_value, skip_memo)?;
    let commitment = decrypted.to_commitment();
    if output_commitment.as_bytes() == commitment.as_bytes() {
        Ok(decrypted)
    } else {
        Err(WalletCryptoError::CommitmentMismatchDecryptedData)
    }
}

pub fn encrypt_data(
    amount: u64,
    mask: &RistrettoSecretKey,
    encryption_key: &RistrettoSecretKey,
    memo: Option<&Memo>,
) -> Result<EncryptedData, WalletCryptoError> {
    let commitment = get_commitment_factory().commit_value(mask, amount).to_byte_type();
    let encrypted_data = encrypt_data_inner(encryption_key, &commitment, amount, mask, memo, None)?;
    Ok(encrypted_data)
}

/// Like [`encrypt_data`], but the caller may supply the XChaCha20-Poly1305 nonce instead of having
/// one drawn at random. When `aead_nonce` is `Some`, those 24 bytes are used verbatim, making the
/// produced [`EncryptedData`] deterministic and reproducible for a given key/value/mask/memo; this
/// is intended for deterministic builders and test vectors. When `aead_nonce` is `None`, the result
/// is identical to [`encrypt_data`] (a fresh random nonce). The encryption itself is unchanged.
pub fn encrypt_data_with_nonce(
    amount: u64,
    mask: &RistrettoSecretKey,
    encryption_key: &RistrettoSecretKey,
    memo: Option<&Memo>,
    aead_nonce: Option<[u8; EncryptedData::SIZE_NONCE]>,
) -> Result<EncryptedData, WalletCryptoError> {
    let commitment = get_commitment_factory().commit_value(mask, amount).to_byte_type();
    let encrypted_data = encrypt_data_inner(encryption_key, &commitment, amount, mask, memo, aead_nonce)?;
    Ok(encrypted_data)
}

pub fn decrypt_data(
    encryption_key: &RistrettoSecretKey,
    commitment: &PedersenCommitmentBytes,
    encrypted_data: &EncryptedData,
    skip_memo: bool,
) -> Result<DecryptedData, WalletCryptoError> {
    let (value, mask, memo) = decrypt_inner(encryption_key, commitment, encrypted_data, skip_memo)?;
    Ok(DecryptedData {
        mask_and_value: MaskAndValue { value, mask },
        memo,
    })
}

fn inner_encrypted_data_kdf_aead(
    encryption_key: &RistrettoSecretKey,
    commitment: &PedersenCommitmentBytes,
) -> EncryptedDataKey {
    let mut aead_key = EncryptedDataKey::from(SafeArray::default());
    DomainSeparatedHasher::<Blake2b<U32>, TransactionSecureNonceKdfDomain>::new_with_label("encrypted_value_and_mask")
        .chain(encryption_key.as_bytes())
        .chain(commitment.as_bytes())
        .finalize_into(GenericArray::from_mut_slice(aead_key.reveal_mut()));
    aead_key
}

const ENCRYPTED_DATA_TAG: &[u8] = b"TARI_AAD_VALUE_AND_MASK_EXTEND_NONCE_VARIANT";

/// A memo's ciphertext is padded up to the next size tier so that the on-chain length only reveals a
/// coarse bucket rather than the exact memo length. Tiers are memo-region sizes (the bytes following
/// value+mask); a no-memo output uses `min_size` (tier 0) and is handled separately. The largest tier
/// equals `EncryptedData::MAX_MEMO_SIZE`, so every encoded memo fits in some tier.
const MEMO_SIZE_TIERS: [usize; 4] = [32, 64, 128, EncryptedData::MAX_MEMO_SIZE];

/// Returns the padded memo-region size (>= `encoded_len`) for the smallest tier that fits the memo.
const fn padded_memo_region_size(encoded_len: usize) -> usize {
    let mut i = 0;
    while i < MEMO_SIZE_TIERS.len() {
        if encoded_len <= MEMO_SIZE_TIERS[i] {
            return MEMO_SIZE_TIERS[i];
        }
        i += 1;
    }
    EncryptedData::MAX_MEMO_SIZE
}

fn encrypt_data_inner(
    encryption_key: &RistrettoSecretKey,
    commitment: &PedersenCommitmentBytes,
    value: u64,
    mask: &RistrettoSecretKey,
    memo: Option<&Memo>,
    injected_nonce: Option<[u8; EncryptedData::SIZE_NONCE]>,
) -> Result<EncryptedData, WalletCryptoError> {
    fn payload_slice_mut(bytes: &mut [u8]) -> &mut [u8] {
        bytes
            .get_mut(EncryptedData::payload_offset()..)
            .expect("invariant violation: bytes length is less than payload_offset")
    }

    fn tag_slice_mut(bytes: &mut [u8]) -> &mut [u8] {
        bytes
            .get_mut(..EncryptedData::SIZE_TAG)
            .expect("invariant violation: tag length is less than payload_offset")
    }

    fn nonce_slice_mut(bytes: &mut [u8]) -> &mut [u8] {
        bytes
            .get_mut(EncryptedData::SIZE_TAG..EncryptedData::SIZE_TAG + EncryptedData::SIZE_NONCE)
            .expect("invariant violation: nonce length is less than payload_offset")
    }

    // Produce the AEAD nonce: use the caller-supplied nonce when present, otherwise draw a secure
    // random one. The random nonce is drawn via `rand`
    // (backed by getrandom 0.4) rather than aead's `OsRng` (rand_core 0.6 -> getrandom 0.2). Both are
    // CSPRNGs, but routing through `rand` keeps wasm32 builds on a single getrandom backend instead of
    // dragging in getrandom 0.2's JS env-detection import shim. See crates/ootle_wasm.
    let nonce_bytes = match injected_nonce {
        Some(bytes) => bytes,
        None => {
            let mut bytes = [0u8; EncryptedData::SIZE_NONCE];
            rand::rng().fill_bytes(&mut bytes);
            bytes
        },
    };
    let nonce = XNonce::from_slice(&nonce_bytes);
    let aead_key = inner_encrypted_data_kdf_aead(encryption_key, commitment);
    let cipher = XChaCha20Poly1305::new(GenericArray::from_slice(aead_key.reveal()));

    // Encode the memo (if any) up front so we can pad to a size tier instead of always reserving the
    // maximum memo size. The decoder is self-describing (tag + length prefix) and discards trailing
    // bytes, so the zero padding is recoverable and only the coarse tier — not the exact memo length —
    // is visible on-chain. A no-memo output stays at `min_size`.
    let encoded_memo = memo
        .map(|m| {
            let mut buf = Vec::new();
            m.encode_into(&mut buf)
                .map_err(|e| WalletCryptoError::FailedEncryptData {
                    details: format!("Failed to encode memo: {}", e),
                })?;
            Ok::<_, WalletCryptoError>(buf)
        })
        .transpose()?;

    let memo_region = encoded_memo
        .as_ref()
        .map(|m| padded_memo_region_size(m.len()))
        .unwrap_or(0);

    // Encode the value and mask
    let mut bytes = vec![0; EncryptedData::min_size() + memo_region];
    let payload_mut = payload_slice_mut(&mut bytes);
    payload_mut
        .get_mut(..EncryptedData::SIZE_VALUE)
        .unwrap()
        .copy_from_slice(value.to_le_bytes().as_ref());
    payload_mut
        .get_mut(EncryptedData::SIZE_VALUE..EncryptedData::SIZE_VALUE + EncryptedData::SIZE_MASK)
        .unwrap()
        .copy_from_slice(mask.as_bytes());

    if let Some(encoded_memo) = encoded_memo {
        payload_mut
            .get_mut(EncryptedData::SIZE_VALUE + EncryptedData::SIZE_MASK..)
            .expect("invariant violation: bytes length is less than SIZE_VALUE + SIZE_MASK")
            .get_mut(..encoded_memo.len())
            .expect("invariant: memo region sized to fit encoded memo")
            .copy_from_slice(&encoded_memo);
    }

    // Encrypt in place
    match cipher.encrypt_in_place_detached(nonce, ENCRYPTED_DATA_TAG, payload_mut) {
        Ok(tag) => {
            tag_slice_mut(&mut bytes).copy_from_slice(&tag);
            nonce_slice_mut(&mut bytes).copy_from_slice(&nonce_bytes);

            Ok(EncryptedData::try_from(bytes).expect("bytes length <= EncryptedData::max_size()"))
        },
        Err(err) => {
            bytes.zeroize();
            Err(err.into())
        },
    }
}

fn decrypt_inner(
    encryption_key: &RistrettoSecretKey,
    commitment: &PedersenCommitmentBytes,
    encrypted_data: &EncryptedData,
    skip_memo: bool,
) -> Result<(u64, RistrettoSecretKey, Option<Memo>), WalletCryptoError> {
    // Extract the tag, nonce, and ciphertext
    let tag = Tag::from_slice(
        encrypted_data
            .tag_slice()
            .ok_or_else(|| WalletCryptoError::FailedDecryptData {
                details: "Failed to get tag slice".to_string(),
            })?,
    );
    let nonce =
        XNonce::from_slice(
            encrypted_data
                .nonce_slice()
                .ok_or_else(|| WalletCryptoError::FailedDecryptData {
                    details: "Failed to get nonce slice".to_string(),
                })?,
        );
    let mut bytes = Zeroizing::new(
        encrypted_data
            .payload_slice()
            .ok_or_else(|| WalletCryptoError::FailedDecryptData {
                details: "Failed to get payload slice".to_string(),
            })?
            .to_vec(),
    );

    // Set up the AEAD
    let aead_key = inner_encrypted_data_kdf_aead(encryption_key, commitment);
    let cipher = XChaCha20Poly1305::new(GenericArray::from_slice(aead_key.reveal()));

    // Decrypt in place
    cipher.decrypt_in_place_detached(nonce, ENCRYPTED_DATA_TAG, bytes.as_mut_slice(), tag)?;

    // Decode the value and mask
    let mut value_bytes = [0u8; EncryptedData::SIZE_VALUE];
    value_bytes.copy_from_slice(bytes.get(..EncryptedData::SIZE_VALUE).ok_or(aead::Error)?);
    let value = u64::from_le_bytes(value_bytes);
    let mask_bytes = bytes
        .get(EncryptedData::SIZE_VALUE..EncryptedData::SIZE_VALUE + EncryptedData::SIZE_MASK)
        .ok_or(aead::Error)?;
    // The sender chooses the plaintext, so an authenticated payload may still carry a non-canonical mask.
    let mask =
        RistrettoSecretKey::from_canonical_bytes(mask_bytes).map_err(|_| WalletCryptoError::FailedDecryptData {
            details: "Decrypted mask is not a canonical scalar".to_string(),
        })?;
    let mut memo_bytes = bytes
        .get(EncryptedData::SIZE_VALUE + EncryptedData::SIZE_MASK..)
        .expect("invariant violation: bytes length is less than SIZE_VALUE + SIZE_MASK");

    let memo = if skip_memo || memo_bytes.is_empty() {
        None
    } else {
        // Note any remaining bytes after memo decoding are discarded
        let memo = Memo::decode_from(&mut memo_bytes).map_err(|e| WalletCryptoError::FailedDecryptData {
            details: format!("Failed to decode memo: {}", e),
        })?;
        Some(memo)
    };
    Ok((value, mask, memo))
}

#[cfg(test)]
mod tests {

    use tari_crypto::{keys::SecretKey, ristretto::RistrettoSecretKey};
    use tari_engine_types::crypto::get_commitment_factory;

    use super::*;

    #[test]
    fn it_encrypts_and_decrypts() {
        let key = RistrettoSecretKey::random(&mut rand::rng());
        let amount = 100;
        let commitment = get_commitment_factory().commit_value(&key, amount).to_byte_type();
        let mask = RistrettoSecretKey::random(&mut rand::rng());
        let encrypted = encrypt_data_inner(&key, &commitment, amount, &mask, None, None).unwrap();

        let (value, msk, memo) = decrypt_inner(&key, &commitment, &encrypted, false).unwrap();
        assert_eq!(value, amount);
        assert_eq!(msk, mask);
        assert!(memo.is_none());
        assert_eq!(encrypted.len(), EncryptedData::min_size());
    }

    #[test]
    fn it_encrypts_and_decrypts_with_memo() {
        let key = RistrettoSecretKey::random(&mut rand::rng());
        let amount = 100;
        let commitment = get_commitment_factory().commit_value(&key, amount).to_byte_type();
        let mask = RistrettoSecretKey::random(&mut rand::rng());
        let memo = Memo::new_message("The quick brown fox jumps over the lazy dog").unwrap();
        let encrypted = encrypt_data_inner(&key, &commitment, amount, &mask, Some(&memo), None).unwrap();
        assert!(!String::from_utf8_lossy(encrypted.as_bytes()).contains("the lazy dog"));

        let (value, msk, decrypted_memo) = decrypt_inner(&key, &commitment, &encrypted, false).unwrap();
        assert_eq!(value, amount);
        assert_eq!(msk, mask);
        assert_eq!(decrypted_memo.unwrap(), memo);

        // Test with bytes
        let memo = Memo::new_bytes([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]).unwrap();
        let encrypted = encrypt_data_inner(&key, &commitment, amount, &mask, Some(&memo), None).unwrap();
        let (value, msk, decrypted_memo) = decrypt_inner(&key, &commitment, &encrypted, false).unwrap();
        assert_eq!(value, amount);
        assert_eq!(msk, mask);
        assert_eq!(decrypted_memo.unwrap(), memo);

        // With empty memo
        let memo = Memo::new_bytes([]).unwrap();
        let encrypted = encrypt_data_inner(&key, &commitment, amount, &mask, Some(&memo), None).unwrap();
        let (value, msk, decrypted_memo) = decrypt_inner(&key, &commitment, &encrypted, false).unwrap();
        assert_eq!(value, amount);
        assert_eq!(msk, mask);
        assert_eq!(decrypted_memo.unwrap(), memo);

        // With max bytes
        let memo = Memo::new_bytes([0u8; Memo::MAX_BYTES_LENGTH]).unwrap();
        let encrypted = encrypt_data_inner(&key, &commitment, amount, &mask, Some(&memo), None).unwrap();
        let (value, msk, decrypted_memo) = decrypt_inner(&key, &commitment, &encrypted, false).unwrap();
        assert_eq!(value, amount);
        assert_eq!(msk, mask);
        assert_eq!(decrypted_memo.unwrap(), memo);

        // With a sender address (no pay ref)
        let memo = Memo::new_sender_address([1u8; 32], [2u8; 32], &[]).unwrap();
        let encrypted = encrypt_data_inner(&key, &commitment, amount, &mask, Some(&memo), None).unwrap();
        let (value, msk, decrypted_memo) = decrypt_inner(&key, &commitment, &encrypted, false).unwrap();
        assert_eq!(value, amount);
        assert_eq!(msk, mask);
        assert_eq!(decrypted_memo.unwrap(), memo);

        // With a sender address that carries a max-length pay ref (lands in the largest tier)
        let memo = Memo::new_sender_address([3u8; 32], [4u8; 32], &[5u8; 64]).unwrap();
        let encrypted = encrypt_data_inner(&key, &commitment, amount, &mask, Some(&memo), None).unwrap();
        let (value, msk, decrypted_memo) = decrypt_inner(&key, &commitment, &encrypted, false).unwrap();
        assert_eq!(value, amount);
        assert_eq!(msk, mask);
        assert_eq!(decrypted_memo.unwrap(), memo);
    }

    #[test]
    fn memo_is_padded_to_size_tiers_not_the_max() {
        let key = RistrettoSecretKey::random(&mut rand::rng());
        let amount = 100;
        let commitment = get_commitment_factory().commit_value(&key, amount).to_byte_type();
        let mask = RistrettoSecretKey::random(&mut rand::rng());

        // "Big deposit" encodes to 13 bytes (tag + len + 11), landing in the 32-byte tier.
        let memo = Memo::new_message("Big deposit").unwrap();
        let encrypted = encrypt_data_inner(&key, &commitment, amount, &mask, Some(&memo), None).unwrap();
        assert_eq!(encrypted.len(), EncryptedData::min_size() + 32);
        assert!(encrypted.len() < EncryptedData::max_size());

        // A memo that overflows a tier rounds up to the next one (here 33 encoded bytes -> 64-byte tier).
        let memo = Memo::new_bytes(vec![0u8; 31]).unwrap();
        let encrypted = encrypt_data_inner(&key, &commitment, amount, &mask, Some(&memo), None).unwrap();
        assert_eq!(encrypted.len(), EncryptedData::min_size() + 64);

        // A maximum-length memo still uses the top tier (== max_size).
        let memo = Memo::new_bytes(vec![0u8; Memo::MAX_BYTES_LENGTH]).unwrap();
        let encrypted = encrypt_data_inner(&key, &commitment, amount, &mask, Some(&memo), None).unwrap();
        assert_eq!(encrypted.len(), EncryptedData::max_size());

        // Round-trip still works through the padding.
        let (value, msk, decoded) = decrypt_inner(&key, &commitment, &encrypted, false).unwrap();
        assert_eq!(value, amount);
        assert_eq!(msk, mask);
        assert_eq!(decoded.unwrap(), memo);
    }

    #[test]
    fn it_decrypts_legacy_max_padded_data() {
        // Backward compatibility: data produced by the old encoder was always padded to max_size.
        // The decoder must still recover the memo from a max-sized buffer.
        let key = RistrettoSecretKey::random(&mut rand::rng());
        let amount = 100;
        let commitment = get_commitment_factory().commit_value(&key, amount).to_byte_type();
        let mask = RistrettoSecretKey::random(&mut rand::rng());
        let memo = Memo::new_message("Big deposit").unwrap();

        // Hand-build a max-sized payload exactly as the old encoder did.
        let nonce_bytes = [7u8; EncryptedData::SIZE_NONCE];
        let nonce = XNonce::from_slice(&nonce_bytes);
        let aead_key = inner_encrypted_data_kdf_aead(&key, &commitment);
        let cipher = XChaCha20Poly1305::new(GenericArray::from_slice(aead_key.reveal()));

        let mut bytes = vec![0u8; EncryptedData::max_size()];
        let payload = &mut bytes[EncryptedData::payload_offset()..];
        payload[..EncryptedData::SIZE_VALUE].copy_from_slice(amount.to_le_bytes().as_ref());
        payload[EncryptedData::SIZE_VALUE..EncryptedData::SIZE_VALUE + EncryptedData::SIZE_MASK]
            .copy_from_slice(mask.as_bytes());
        let mut memo_slice = &mut payload[EncryptedData::SIZE_VALUE + EncryptedData::SIZE_MASK..];
        memo.encode_into(&mut memo_slice).unwrap();

        let payload = &mut bytes[EncryptedData::payload_offset()..];
        let tag = cipher
            .encrypt_in_place_detached(nonce, ENCRYPTED_DATA_TAG, payload)
            .unwrap();
        bytes[..EncryptedData::SIZE_TAG].copy_from_slice(&tag);
        bytes[EncryptedData::SIZE_TAG..EncryptedData::SIZE_TAG + EncryptedData::SIZE_NONCE]
            .copy_from_slice(&nonce_bytes);

        let legacy = EncryptedData::try_from(bytes).unwrap();
        assert_eq!(legacy.len(), EncryptedData::max_size());

        let (value, msk, decoded) = decrypt_inner(&key, &commitment, &legacy, false).unwrap();
        assert_eq!(value, amount);
        assert_eq!(msk, mask);
        assert_eq!(decoded.unwrap(), memo);
    }

    #[test]
    fn it_always_returns_a_none_memo_if_skip_memo_is_true() {
        let key = RistrettoSecretKey::random(&mut rand::rng());
        let amount = 100;
        let commitment = get_commitment_factory().commit_value(&key, amount).to_byte_type();
        let mask = RistrettoSecretKey::random(&mut rand::rng());
        let memo = Memo::new_message("The quick brown fox jumps over the lazy dog").unwrap();
        let encrypted = encrypt_data_inner(&key, &commitment, amount, &mask, Some(&memo), None).unwrap();

        let (value, msk, decrypted_memo) = decrypt_inner(&key, &commitment, &encrypted, true).unwrap();
        assert_eq!(value, amount);
        assert_eq!(msk, mask);
        assert!(decrypted_memo.is_none());
    }

    #[test]
    fn it_rejects_an_authenticated_payload_with_a_non_canonical_mask() {
        let key = RistrettoSecretKey::random(&mut rand::rng());
        let amount = 100u64;
        let mask = RistrettoSecretKey::random(&mut rand::rng());
        let commitment = get_commitment_factory().commit_value(&mask, amount).to_byte_type();

        // A validly sealed value||mask payload whose mask bytes exceed the group order.
        let nonce_bytes = [9u8; EncryptedData::SIZE_NONCE];
        let nonce = XNonce::from_slice(&nonce_bytes);
        let aead_key = inner_encrypted_data_kdf_aead(&key, &commitment);
        let cipher = XChaCha20Poly1305::new(GenericArray::from_slice(aead_key.reveal()));

        let mut bytes = vec![0u8; EncryptedData::min_size()];
        let payload = &mut bytes[EncryptedData::payload_offset()..];
        payload[..EncryptedData::SIZE_VALUE].copy_from_slice(&amount.to_le_bytes());
        payload[EncryptedData::SIZE_VALUE..].copy_from_slice(&[0xFF; EncryptedData::SIZE_MASK]);
        let tag = cipher
            .encrypt_in_place_detached(nonce, ENCRYPTED_DATA_TAG, payload)
            .unwrap();
        bytes[..EncryptedData::SIZE_TAG].copy_from_slice(&tag);
        bytes[EncryptedData::SIZE_TAG..EncryptedData::payload_offset()].copy_from_slice(&nonce_bytes);
        let encrypted = EncryptedData::try_from(bytes).unwrap();

        let err = decrypt_data(&key, &commitment, &encrypted, false).unwrap_err();
        assert!(matches!(err, WalletCryptoError::FailedDecryptData { .. }), "{err:?}");
        let err = unblind_output(&commitment, &encrypted, &key, false).unwrap_err();
        assert!(matches!(err, WalletCryptoError::FailedDecryptData { .. }), "{err:?}");
    }
}
