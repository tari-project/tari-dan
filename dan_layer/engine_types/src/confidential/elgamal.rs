//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::convert;

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
        lookup_table: &mut TLookup,
    ) -> Result<Option<u64>, TLookup::Error> {
        let mut result = Self::batched_brute_force(view_private_key, value_range, lookup_table, Some(self))?;
        Ok(result.pop().and_then(convert::identity))
    }

    pub fn batched_brute_force<'a, IValueRange, TLookup, IBalances>(
        view_private_key: &PrivateKey,
        value_range: IValueRange,
        lookup_table: &mut TLookup,
        verifiable_balances: IBalances,
    ) -> Result<Vec<Option<u64>>, TLookup::Error>
    where
        IValueRange: IntoIterator<Item = u64>,
        TLookup: ValueLookupTable,
        IBalances: IntoIterator<Item = &'a Self>,
    {
        let mut balances = verifiable_balances
            .into_iter()
            .enumerate()
            .map(|(i, balance)| {
                // V = E - pR
                let balance = &balance.encrypted - view_private_key * &balance.public_nonce;
                (i, copy_fixed(balance.as_bytes()))
            })
            .collect::<Vec<_>>();

        let mut results = vec![None; balances.len()];

        for v in value_range {
            let value = lookup_table.lookup(v)?.unwrap_or_else(|| {
                let pk = PublicKey::from_secret_key(&PrivateKey::from(v));
                copy_fixed(pk.as_bytes())
            });

            while let Some(pos) = balances.iter().position(|(_, balance)| value == *balance) {
                let (order, _) = balances.swap_remove(pos);
                results.get_mut(order).unwrap().replace(v);
            }

            if balances.is_empty() {
                break;
            }
        }

        Ok(results)
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
                .brute_force_balance(view_sk, 0..=10000, &mut TestLookupTable)
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

            let balance = subject
                .brute_force_balance(view_sk, 0..=10, &mut TestLookupTable)
                .unwrap();
            assert_eq!(balance, Some(10));

            let balance = subject
                .brute_force_balance(view_sk, 10..=12, &mut TestLookupTable)
                .unwrap();
            assert_eq!(balance, Some(10));
        }

        #[test]
        fn it_returns_none_if_the_value_out_of_range() {
            let subject = ElgamalVerifiableBalance {
                encrypted: PublicKey::from_secret_key(&PrivateKey::from(101)),
                public_nonce: Default::default(),
            };

            let balance = subject
                .brute_force_balance(&PrivateKey::default(), 0..=100, &mut TestLookupTable)
                .unwrap();
            assert_eq!(balance, None);

            let balance = subject
                .brute_force_balance(&PrivateKey::default(), 102..=103, &mut TestLookupTable)
                .unwrap();
            assert_eq!(balance, None);
        }

        #[test]
        fn it_brute_forces_a_batch() {
            let view_sk = &PrivateKey::random(&mut OsRng);

            let subject = (0..100)
                .map(|v| {
                    let (nonce_sk, nonce_pk) = PublicKey::random_keypair(&mut OsRng);
                    let rp = nonce_sk * view_sk;
                    ElgamalVerifiableBalance {
                        encrypted: PublicKey::from_secret_key(&rp) + PublicKey::from_secret_key(&PrivateKey::from(v)),
                        public_nonce: nonce_pk,
                    }
                })
                .collect::<Vec<_>>();

            let balances =
                ElgamalVerifiableBalance::batched_brute_force(view_sk, 0..=10000, &mut TestLookupTable, subject.iter())
                    .unwrap();
            assert_eq!(balances.len(), 100);
            for (i, balance) in balances.into_iter().enumerate() {
                assert_eq!(balance, Some(i as u64));
            }
        }
    }
}
