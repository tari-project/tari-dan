//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use borsh::BorshSerialize;
use ootle_byte_type::{ConvertFromByteType, ToByteType};
use tari_bor::{Deserialize, Serialize};
use tari_crypto::{
    commitment::HomomorphicCommitmentFactory,
    keys::{PublicKey, SecretKey},
    ristretto::{RistrettoPublicKey, RistrettoSecretKey, pedersen::PedersenCommitment},
    tari_utilities,
    tari_utilities::ByteArray,
};
use tari_template_lib::types::{crypto::RistrettoPublicKeyBytes, stealth::ViewableBalanceProof};

use crate::{
    crypto::{get_commitment_factory, messages, value_lookup::ValueLookup},
    resource_container::ResourceError,
};

pub fn validate_elgamal_verifiable_balance_proof(
    commitment: &PedersenCommitment,
    view_key: Option<&RistrettoPublicKey>,
    viewable_balance_proof: Option<&ViewableBalanceProof>,
) -> Result<Option<ElgamalVerifiableBalance>, ResourceError> {
    // Check that if a view key is provided, then a viewable balance proof is also provided and vice versa
    let Some(view_key) = view_key else {
        if viewable_balance_proof.is_none() {
            return Ok(None);
        }
        return Err(ResourceError::InvalidConfidentialProof {
            details: "ViewableBalanceProof provided for a resource that is not viewable".to_string(),
        });
    };

    let Some(proof) = viewable_balance_proof else {
        return Err(ResourceError::InvalidConfidentialProof {
            details: "ViewableBalanceProof is required for a viewable resource".to_string(),
        });
    };

    // Decode and check that each field is well-formed
    let encrypted = RistrettoPublicKey::from_canonical_bytes(&*proof.elgamal_encrypted).map_err(|_| {
        ResourceError::InvalidConfidentialProof {
            details: "Invalid value for E".to_string(),
        }
    })?;

    if proof.elgamal_public_nonce.is_zero() {
        return Err(ResourceError::InvalidConfidentialProof {
            details: "Public nonce for ElGamal encryption cannot be the identity point".to_string(),
        });
    }

    let elgamal_public_nonce =
        RistrettoPublicKey::from_canonical_bytes(&*proof.elgamal_public_nonce).map_err(|_| {
            ResourceError::InvalidConfidentialProof {
                details: "Invalid public key for R".to_string(),
            }
        })?;

    let c_prime = PedersenCommitment::from_canonical_bytes(&*proof.c_prime).map_err(|_| {
        ResourceError::InvalidConfidentialProof {
            details: "Invalid commitment for C'".to_string(),
        }
    })?;

    let e_prime = PedersenCommitment::from_canonical_bytes(&*proof.e_prime).map_err(|_| {
        ResourceError::InvalidConfidentialProof {
            details: "Invalid commitment for E'".to_string(),
        }
    })?;

    let r_prime = RistrettoPublicKey::from_canonical_bytes(&*proof.r_prime).map_err(|_| {
        ResourceError::InvalidConfidentialProof {
            details: "Invalid public key for R'".to_string(),
        }
    })?;

    let s_v =
        RistrettoSecretKey::from_canonical_bytes(&*proof.s_v).map_err(|_| ResourceError::InvalidConfidentialProof {
            details: "Invalid private key for s_v".to_string(),
        })?;

    let s_m =
        RistrettoSecretKey::from_canonical_bytes(&*proof.s_m).map_err(|_| ResourceError::InvalidConfidentialProof {
            details: "Invalid private key for s_m".to_string(),
        })?;

    let s_r = &RistrettoSecretKey::from_canonical_bytes(&*proof.s_r).map_err(|_| {
        ResourceError::InvalidConfidentialProof {
            details: "Invalid private key for s_r".to_string(),
        }
    })?;

    // Fiat-Shamir challenge
    let e = &RistrettoSecretKey::from_uniform_bytes(&messages::viewable_balance_proof64(
        commitment,
        view_key,
        proof.as_challenge_fields()
    ))
        // TODO: it would be better if from_uniform_bytes took a [u8; 64]
        .expect("INVARIANT VIOLATION: RistrettoSecretKey::from_uniform_bytes and hash output length mismatch");

    // Check eC + C' ?= s_m.G + sv.H
    let left = e * commitment.as_public_key() + c_prime.as_public_key();
    let right = get_commitment_factory().commit(&s_m, &s_v);
    if left != *right.as_public_key() {
        return Err(ResourceError::InvalidConfidentialProof {
            details: "Invalid viewable balance proof (eC + C' != s_m.G + s_v.H)".to_string(),
        });
    }

    // Check eE + E' ?= s_v.G + s_r.P
    let left = e * &encrypted + e_prime.as_public_key();
    let right = RistrettoPublicKey::from_secret_key(&s_v) + s_r * view_key;
    if left != right {
        return Err(ResourceError::InvalidConfidentialProof {
            details: "Invalid viewable balance proof (eE + E' != s_v.G + s_r.P)".to_string(),
        });
    }

    // Check eR + R' ?= s_r.G
    let left = e * &elgamal_public_nonce + r_prime;
    let right = RistrettoPublicKey::from_secret_key(s_r);
    if left != right {
        return Err(ResourceError::InvalidConfidentialProof {
            details: "Invalid viewable balance proof (eR + R' != s_r.G)".to_string(),
        });
    }

    Ok(Some(ElgamalVerifiableBalance {
        encrypted,
        public_nonce: elgamal_public_nonce,
    }))
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    minicbor::Encode,
    minicbor::Decode,
    minicbor::CborLen,
    Serialize,
    Deserialize,
    BorshSerialize,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ElgamalVerifiableBalanceBytes {
    #[n(0)]
    pub encrypted: RistrettoPublicKeyBytes,
    #[n(1)]
    pub public_nonce: RistrettoPublicKeyBytes,
}

impl ConvertFromByteType<ElgamalVerifiableBalanceBytes> for ElgamalVerifiableBalance {
    type Error = tari_utilities::ByteArrayError;

    fn convert_from_byte_type(bytes: &ElgamalVerifiableBalanceBytes) -> Result<Self, Self::Error> {
        let encrypted = RistrettoPublicKey::convert_from_byte_type(&bytes.encrypted)?;
        let public_nonce = RistrettoPublicKey::convert_from_byte_type(&bytes.public_nonce)?;
        Ok(ElgamalVerifiableBalance {
            encrypted,
            public_nonce,
        })
    }
}

impl From<ElgamalVerifiableBalance> for ElgamalVerifiableBalanceBytes {
    fn from(value: ElgamalVerifiableBalance) -> Self {
        (&value).into()
    }
}

impl From<&ElgamalVerifiableBalance> for ElgamalVerifiableBalanceBytes {
    fn from(value: &ElgamalVerifiableBalance) -> Self {
        Self {
            encrypted: value.encrypted.to_byte_type(),
            public_nonce: value.public_nonce.to_byte_type(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElgamalVerifiableBalance {
    pub encrypted: RistrettoPublicKey,
    pub public_nonce: RistrettoPublicKey,
}

impl ElgamalVerifiableBalance {
    /// The compressed point `V = E - p·R`, which the value lookup table is reverse-searched for to recover the
    /// balance `v` (the table maps `v -> v·G`).
    pub fn value_lookup_target(&self, view_private_key: &RistrettoSecretKey) -> RistrettoPublicKeyBytes {
        let point = &self.encrypted - view_private_key * &self.public_nonce;
        point.to_byte_type()
    }

    /// Decrypts this viewable balance, recovering `v` by reverse-looking-up its value point `V = E - p·R`.
    pub fn decrypt<L: ValueLookup>(
        &self,
        view_private_key: &RistrettoSecretKey,
        lookup: &L,
    ) -> Result<Option<u64>, L::Error> {
        lookup.lookup(&self.value_lookup_target(view_private_key))
    }

    /// Decrypts many viewable balances at once, returning results aligned to `verifiable_balances`.
    pub fn decrypt_many<'a, L, IBalances>(
        view_private_key: &RistrettoSecretKey,
        verifiable_balances: IBalances,
        lookup: &L,
    ) -> Result<Vec<Option<u64>>, L::Error>
    where
        L: ValueLookup,
        IBalances: IntoIterator<Item = &'a Self>,
    {
        let targets = verifiable_balances
            .into_iter()
            .map(|balance| balance.value_lookup_target(view_private_key))
            .collect::<Vec<_>>();
        lookup.lookup_many(&targets)
    }
}

impl TryFrom<&ElgamalVerifiableBalanceBytes> for ElgamalVerifiableBalance {
    type Error = tari_utilities::ByteArrayError;

    fn try_from(value: &ElgamalVerifiableBalanceBytes) -> Result<Self, Self::Error> {
        let encrypted = RistrettoPublicKey::convert_from_byte_type(&value.encrypted)?;
        let public_nonce = RistrettoPublicKey::convert_from_byte_type(&value.public_nonce)?;
        Ok(ElgamalVerifiableBalance {
            encrypted,
            public_nonce,
        })
    }
}

impl ToByteType for ElgamalVerifiableBalance {
    type ByteType = ElgamalVerifiableBalanceBytes;

    fn to_byte_type(&self) -> Self::ByteType {
        ElgamalVerifiableBalanceBytes {
            encrypted: self.encrypted.to_byte_type(),
            public_nonce: self.public_nonce.to_byte_type(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{convert::Infallible, ops::RangeInclusive};

    use tari_crypto::{
        keys::{PublicKey, SecretKey},
        ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    };

    use super::*;

    /// Reverse value lookup that scans a range, computing `v·G` for each candidate — the `GenerateValueLookup`
    /// behaviour, kept local so `engine_types` does not depend on `tari_ootle_wallet_crypto`.
    struct TestValueLookup {
        range: RangeInclusive<u64>,
    }

    impl ValueLookup for TestValueLookup {
        type Error = Infallible;

        fn lookup(&self, point: &RistrettoPublicKeyBytes) -> Result<Option<u64>, Self::Error> {
            for v in self.range.clone() {
                if RistrettoPublicKey::from_secret_key(&RistrettoSecretKey::from(v)).to_byte_type() == *point {
                    return Ok(Some(v));
                }
            }
            Ok(None)
        }
    }

    mod decrypt {

        use super::*;

        #[test]
        fn it_finds_the_value() {
            const VALUE: u64 = 5242;
            let mut rng = rand::rng();
            let view_sk = &RistrettoSecretKey::random(&mut rng);
            let (nonce_sk, nonce_pk) = RistrettoPublicKey::random_keypair(&mut rng);

            let rp = nonce_sk * view_sk;

            let subject = ElgamalVerifiableBalance {
                encrypted: RistrettoPublicKey::from_secret_key(&rp) +
                    RistrettoPublicKey::from_secret_key(&RistrettoSecretKey::from(VALUE)),
                public_nonce: nonce_pk,
            };

            let balance = subject.decrypt(view_sk, &TestValueLookup { range: 0..=10000 }).unwrap();
            assert_eq!(balance, Some(VALUE));
        }

        #[test]
        fn it_returns_the_value_equal_to_max_value() {
            let mut rng = rand::rng();
            let view_sk = &RistrettoSecretKey::random(&mut rng);
            let (nonce_sk, nonce_pk) = RistrettoPublicKey::random_keypair(&mut rng);

            let rp = nonce_sk * view_sk;

            let subject = ElgamalVerifiableBalance {
                encrypted: RistrettoPublicKey::from_secret_key(&rp) +
                    RistrettoPublicKey::from_secret_key(&RistrettoSecretKey::from(10u64)),
                public_nonce: nonce_pk,
            };

            let balance = subject.decrypt(view_sk, &TestValueLookup { range: 0..=10 }).unwrap();
            assert_eq!(balance, Some(10));

            let balance = subject.decrypt(view_sk, &TestValueLookup { range: 10..=12 }).unwrap();
            assert_eq!(balance, Some(10));
        }

        #[test]
        fn it_returns_none_if_the_value_out_of_range() {
            let subject = ElgamalVerifiableBalance {
                encrypted: RistrettoPublicKey::from_secret_key(&RistrettoSecretKey::from(101u64)),
                public_nonce: Default::default(),
            };

            let balance = subject
                .decrypt(&RistrettoSecretKey::default(), &TestValueLookup { range: 0..=100 })
                .unwrap();
            assert_eq!(balance, None);

            let balance = subject
                .decrypt(&RistrettoSecretKey::default(), &TestValueLookup { range: 102..=103 })
                .unwrap();
            assert_eq!(balance, None);
        }

        #[test]
        fn it_decrypts_a_batch() {
            let mut rng = rand::rng();
            let view_sk = &RistrettoSecretKey::random(&mut rng);

            let subject = (0u64..100)
                .map(|v| {
                    let (nonce_sk, nonce_pk) = RistrettoPublicKey::random_keypair(&mut rng);
                    let rp = nonce_sk * view_sk;
                    ElgamalVerifiableBalance {
                        encrypted: RistrettoPublicKey::from_secret_key(&rp) +
                            RistrettoPublicKey::from_secret_key(&RistrettoSecretKey::from(v)),
                        public_nonce: nonce_pk,
                    }
                })
                .collect::<Vec<_>>();

            let balances =
                ElgamalVerifiableBalance::decrypt_many(view_sk, subject.iter(), &TestValueLookup { range: 0..=10000 })
                    .unwrap();
            assert_eq!(balances.len(), 100);
            for (i, balance) in balances.into_iter().enumerate() {
                assert_eq!(balance, Some(i as u64));
            }
        }
    }
}
