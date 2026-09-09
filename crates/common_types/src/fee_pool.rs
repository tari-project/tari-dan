//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib_types::{ValidatorFeePoolAddress, crypto::RistrettoPublicKeyBytes};

use crate::{NumPreshards, shard::Shard};

/// Derives a deterministic [`ValidatorFeePoolAddress`] that lives in `shard` when partitioned into `num_preshards`.
///
/// Layout: two shard-prefix bytes followed by 30 bytes of the public key. The top `log2(num_preshards)` bits of the
/// prefix encode `shard - 1`; the remaining prefix bits are zero so they are stable if `NumPreshards` is later
/// grown — every existing pool then migrates to the lower-numbered child shard rather than fanning out by pk bits.
/// Reserving two bytes lets `NumPreshards` grow to at most `2^16` without changing the layout.
///
/// `shard` must be in `1..=num_preshards`: shard 0 is the global shard and has no index, and an index that does not
/// fit in `shard_bits` would wrap into another shard's prefix, silently yielding an address in the wrong shard.
pub fn derive_fee_pool_address(
    public_key_bytes: &RistrettoPublicKeyBytes,
    num_preshards: NumPreshards,
    shard: Shard,
) -> Result<ValidatorFeePoolAddress, InvalidFeePoolShard> {
    if shard.is_global() || shard.as_u32() > num_preshards.as_u32() {
        return Err(InvalidFeePoolShard { shard, num_preshards });
    }
    // For num_preshards = 256, log2(256) = 8
    let shard_bits = num_preshards.as_u32().trailing_zeros();
    // shift required to place the shard index in the top `shard_bits` of the 2-byte prefix
    let shift = u16::BITS - shard_bits;
    // shard 0 is global, so shard is count-based; convert to an index
    let shard_index = shard.as_u32() - 1;
    let prefix = (shard_index << shift) as u16;

    let mut address = [0u8; 32];
    address[..2].copy_from_slice(&prefix.to_be_bytes());
    address[2..].copy_from_slice(&public_key_bytes[2..]);
    Ok(ValidatorFeePoolAddress::from_array(address))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Shard {shard} cannot hold a validator fee pool: expected a shard in 1..={}", .num_preshards.as_u32())]
pub struct InvalidFeePoolShard {
    pub shard: Shard,
    pub num_preshards: NumPreshards,
}

#[cfg(test)]
mod tests {
    use ootle_byte_type::ToByteType;
    use tari_crypto::{keys::PublicKey, ristretto::RistrettoPublicKey};

    use super::*;
    use crate::SubstateAddress;

    fn assert_pool_lands_in_shard(pk: &RistrettoPublicKeyBytes, num_preshards: NumPreshards, shard: Shard) {
        let fee_pool_address = derive_fee_pool_address(pk, num_preshards, shard).unwrap();
        let addr = SubstateAddress::from_substate_id(&fee_pool_address.into(), 0);
        assert_eq!(addr.to_shard(num_preshards), shard);
    }

    #[test]
    fn it_creates_a_pool_address_that_naturally_falls_in_the_shard() {
        let pk = RistrettoPublicKeyBytes::from([0xff; 32]);
        assert_pool_lands_in_shard(&pk, NumPreshards::P256, Shard::from(1));

        let (_, pk) = RistrettoPublicKey::random_keypair(&mut rand::rng());
        assert_pool_lands_in_shard(&pk.to_byte_type(), NumPreshards::P256, Shard::from(212));
    }

    #[test]
    fn it_works_for_shard_256_and_other_n() {
        let pk = RistrettoPublicKeyBytes::from([0xff; 32]);
        assert_pool_lands_in_shard(&pk, NumPreshards::P256, Shard::from(256));
        assert_pool_lands_in_shard(&pk, NumPreshards::P128, Shard::from(128));
        assert_pool_lands_in_shard(&pk, NumPreshards::P64, Shard::from(50));
        assert_pool_lands_in_shard(&pk, NumPreshards::P2, Shard::from(2));
    }

    #[test]
    fn it_preserves_pk_entropy_beyond_the_shard_prefix() {
        // Two pks differing only in bytes 2.. should produce different addresses but the same shard.
        let mut pk_a = [0u8; 32];
        let mut pk_b = [0u8; 32];
        pk_a[5] = 0xAB;
        pk_b[5] = 0xCD;
        let pk_a = RistrettoPublicKeyBytes::from(pk_a);
        let pk_b = RistrettoPublicKeyBytes::from(pk_b);

        let addr_a = derive_fee_pool_address(&pk_a, NumPreshards::P256, Shard::from(42)).unwrap();
        let addr_b = derive_fee_pool_address(&pk_b, NumPreshards::P256, Shard::from(42)).unwrap();
        assert_ne!(addr_a, addr_b);
        assert_pool_lands_in_shard(&pk_a, NumPreshards::P256, Shard::from(42));
        assert_pool_lands_in_shard(&pk_b, NumPreshards::P256, Shard::from(42));
    }

    #[test]
    fn it_rejects_shards_outside_the_preshard_range() {
        let pk = RistrettoPublicKeyBytes::from([0xff; 32]);
        for (num_preshards, shard) in [
            (NumPreshards::P256, Shard::global()),
            (NumPreshards::P256, Shard::from(257)),
            (NumPreshards::P2, Shard::from(3)),
            (NumPreshards::P64, Shard::from(u32::MAX)),
        ] {
            let err = derive_fee_pool_address(&pk, num_preshards, shard).unwrap_err();
            assert_eq!(err, InvalidFeePoolShard { shard, num_preshards });
        }
    }

    #[test]
    fn it_migrates_predictably_under_a_finer_split() {
        // A pool derived for shard X of P_k must land in the lower-numbered child (shard 2X-1) of P_{k+1},
        // regardless of pk. This relies on the prefix bits below the current shard-bits being zero.
        let pk = RistrettoPublicKeyBytes::from([0xff; 32]);
        for shard in [1u32, 17, 64, 128, 256] {
            let addr = derive_fee_pool_address(&pk, NumPreshards::P256, Shard::from(shard)).unwrap();
            // P256 has no finer NumPreshards variant yet, so simulate the split by reading top 9 bits manually:
            // under a hypothetical P512, the shard index would be (byte0 << 1) | (byte1 >> 7).
            let bytes = SubstateAddress::from_substate_id(&addr.into(), 0).into_array();
            let p512_index = (u32::from(bytes[0]) << 1) | u32::from(bytes[1] >> 7);
            assert_eq!(p512_index + 1, 2 * shard - 1, "shard {shard}");
        }
    }
}
