//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_bor::{Deserialize, Serialize};
use tari_common_types::types::{PrivateKey, PublicKey};
use tari_crypto::keys::PublicKey as _;
use tari_utilities::ByteArray;

use crate::confidential::value_lookup_table::ValueLookupTable;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct ElgamalVerifiableBalance {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub encrypted: PublicKey,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_nonce: PublicKey,
}

impl ElgamalVerifiableBalance {
    pub fn brute_force_balance<I: IntoIterator<Item = u64>, TLookup: ValueLookupTable>(
        &self,
        view_private_key: &PrivateKey,
        value_range: I,
        mut lookup_table: TLookup,
    ) -> Result<Option<u64>, TLookup::Error> {
        // V = E - pR
        let balance = &self.encrypted - view_private_key * &self.public_nonce;
        let balance_bytes = copy_fixed(balance.as_bytes());

        for v in value_range {
            let value = lookup_table.lookup(v)?.unwrap_or_else(|| {
                let pk = PublicKey::from_secret_key(&PrivateKey::from(v));
                copy_fixed(pk.as_bytes())
            });
            if value == balance_bytes {
                return Ok(Some(v));
            }
        }

        Ok(None)
    }
}

fn copy_fixed(src: &[u8]) -> [u8; 32] {
    let mut buf = [0u8; 32];
    buf.copy_from_slice(src);
    buf
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use rand::rngs::OsRng;
    use tari_crypto::keys::SecretKey;

    use super::*;

    #[derive(Default)]
    pub struct TestLookupTable;

    impl ValueLookupTable for TestLookupTable {
        type Error = Infallible;

        fn lookup(&mut self, value: u64) -> Result<Option<[u8; 32]>, Self::Error> {
            // This would be a sequential lookup in a real implementation
            Ok(Some(copy_fixed(
                PublicKey::from_secret_key(&PrivateKey::from(value)).as_bytes(),
            )))
        }
    }

    mod brute_force_balance {
        use super::*;

        #[test]
        fn it_finds_the_value() {
            const VALUE: u64 = 5242;
            let view_sk = &PrivateKey::random(&mut OsRng);
            let (nonce_sk, nonce_pk) = PublicKey::random_keypair(&mut OsRng);

            let rp = nonce_sk * view_sk;

            let subject = ElgamalVerifiableBalance {
                encrypted: PublicKey::from_secret_key(&rp) + PublicKey::from_secret_key(&PrivateKey::from(VALUE)),
                public_nonce: nonce_pk,
            };

            let balance = subject
                .brute_force_balance(view_sk, 0..=10000, TestLookupTable)
                .unwrap();
            assert_eq!(balance, Some(VALUE));
        }

        #[test]
        fn it_returns_the_value_equal_to_max_value() {
            let view_sk = &PrivateKey::random(&mut OsRng);
            let (nonce_sk, nonce_pk) = PublicKey::random_keypair(&mut OsRng);

            let rp = nonce_sk * view_sk;

            let subject = ElgamalVerifiableBalance {
                encrypted: PublicKey::from_secret_key(&rp) + PublicKey::from_secret_key(&PrivateKey::from(10)),
                public_nonce: nonce_pk,
            };

            let balance = subject.brute_force_balance(view_sk, 0..=10, TestLookupTable).unwrap();
            assert_eq!(balance, Some(10));

            let balance = subject.brute_force_balance(view_sk, 10..=12, TestLookupTable).unwrap();
            assert_eq!(balance, Some(10));
        }

        #[test]
        fn it_returns_none_if_the_value_out_of_range() {
            let subject = ElgamalVerifiableBalance {
                encrypted: PublicKey::from_secret_key(&PrivateKey::from(101)),
                public_nonce: Default::default(),
            };

            let balance = subject
                .brute_force_balance(&PrivateKey::default(), 0..=100, TestLookupTable)
                .unwrap();
            assert_eq!(balance, None);

            let balance = subject
                .brute_force_balance(&PrivateKey::default(), 102..=103, TestLookupTable)
                .unwrap();
            assert_eq!(balance, None);
        }
    }
}
