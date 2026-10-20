//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

/// The key a transaction acts as wherever the engine needs a single signer: `OwnerRule::OwnedBySigner`,
/// `CallerContext::transaction_signer_public_key`, `CallerContext::get_main_signer_proof` and the author of a
/// published template.
///
/// Every authorization signature and the seal signature cover this field, so all signers agree on who the main
/// signer is.
///
/// Encoding: `None` and `SealSigner` encode as the booleans `false` and `true` in both CBOR and borsh (whose variant
/// tag is the same byte), and `Authorizer` as the key, so a transaction carrying the boolean
/// `is_seal_signer_authorized` decodes, hashes and verifies as the matching variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, borsh::BorshSerialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum MainSigner {
    /// The seal signer is not authorized and the transaction has no main signer. Anything that needs one aborts.
    None,
    /// The seal signer is authorized and is the main signer.
    #[default]
    SealSigner,
    /// The seal signer is not authorized and the main signer is this key, which must be one of the transaction's
    /// authorization signers.
    Authorizer(RistrettoPublicKeyBytes),
}

impl MainSigner {
    pub fn is_seal_signer_authorized(&self) -> bool {
        matches!(self, Self::SealSigner)
    }

    pub fn authorizer(&self) -> Option<&RistrettoPublicKeyBytes> {
        match self {
            Self::Authorizer(public_key) => Some(public_key),
            Self::None | Self::SealSigner => None,
        }
    }
}

impl<C> minicbor::Encode<C> for MainSigner {
    fn encode<W: minicbor::encode::Write>(
        &self,
        e: &mut minicbor::Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), minicbor::encode::Error<W::Error>> {
        match self {
            Self::None => {
                e.bool(false)?;
            },
            Self::SealSigner => {
                e.bool(true)?;
            },
            Self::Authorizer(public_key) => public_key.encode(e, ctx)?,
        }
        Ok(())
    }
}

impl<'b, C> minicbor::Decode<'b, C> for MainSigner {
    fn decode(d: &mut minicbor::Decoder<'b>, ctx: &mut C) -> Result<Self, minicbor::decode::Error> {
        if d.datatype()? == minicbor::data::Type::Bool {
            return Ok(if d.bool()? { Self::SealSigner } else { Self::None });
        }
        Ok(Self::Authorizer(RistrettoPublicKeyBytes::decode(d, ctx)?))
    }
}

impl<C> minicbor::CborLen<C> for MainSigner {
    fn cbor_len(&self, ctx: &mut C) -> usize {
        match self {
            Self::None | Self::SealSigner => 1,
            Self::Authorizer(public_key) => public_key.cbor_len(ctx),
        }
    }
}

/// Every mode other than `main_signer`, for tests asserting that the field is committed to.
#[cfg(test)]
pub(crate) fn other_main_signers(main_signer: MainSigner) -> Vec<MainSigner> {
    let authorizer = RistrettoPublicKeyBytes::from_bytes(&[9u8; 32]).expect("32 bytes");
    let other_authorizer = RistrettoPublicKeyBytes::from_bytes(&[10u8; 32]).expect("32 bytes");
    [
        MainSigner::None,
        MainSigner::SealSigner,
        MainSigner::Authorizer(authorizer),
        MainSigner::Authorizer(other_authorizer),
    ]
    .into_iter()
    .filter(|other| *other != main_signer)
    .collect()
}

#[cfg(feature = "serde")]
mod serde_impl {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

    use super::MainSigner;

    #[derive(Serialize, Deserialize)]
    enum Tagged {
        None,
        SealSigner,
        Authorizer(RistrettoPublicKeyBytes),
    }

    /// Also accepts the boolean `is_seal_signer_authorized`.
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Repr {
        Flag(bool),
        Tagged(Tagged),
    }

    impl Serialize for MainSigner {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            match *self {
                Self::None => Tagged::None,
                Self::SealSigner => Tagged::SealSigner,
                Self::Authorizer(public_key) => Tagged::Authorizer(public_key),
            }
            .serialize(serializer)
        }
    }

    impl<'de> Deserialize<'de> for MainSigner {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            Ok(match Repr::deserialize(deserializer)? {
                Repr::Flag(true) | Repr::Tagged(Tagged::SealSigner) => Self::SealSigner,
                Repr::Flag(false) | Repr::Tagged(Tagged::None) => Self::None,
                Repr::Tagged(Tagged::Authorizer(public_key)) => Self::Authorizer(public_key),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use borsh::BorshSerialize;

    use super::*;

    fn borsh_bytes<T: BorshSerialize>(value: &T) -> Vec<u8> {
        borsh::to_vec(value).unwrap()
    }

    #[test]
    fn borsh_matches_the_boolean_flag() {
        assert_eq!(borsh_bytes(&MainSigner::None), borsh_bytes(&false));
        assert_eq!(borsh_bytes(&MainSigner::SealSigner), borsh_bytes(&true));
    }

    #[test]
    fn cbor_decodes_the_boolean_flag() {
        let decoded: MainSigner = tari_bor::decode(&tari_bor::encode(&false).unwrap()).unwrap();
        assert_eq!(decoded, MainSigner::None);
        let decoded: MainSigner = tari_bor::decode(&tari_bor::encode(&true).unwrap()).unwrap();
        assert_eq!(decoded, MainSigner::SealSigner);
    }

    #[test]
    fn cbor_round_trips() {
        let public_key = RistrettoPublicKeyBytes::from_bytes(&[7u8; 32]).unwrap();
        for value in [
            MainSigner::None,
            MainSigner::SealSigner,
            MainSigner::Authorizer(public_key),
        ] {
            let encoded = tari_bor::encode(&value).unwrap();
            assert_eq!(encoded.len(), minicbor::len(value));
            let decoded: MainSigner = tari_bor::decode(&encoded).unwrap();
            assert_eq!(decoded, value);
        }
        assert_eq!(
            tari_bor::encode(&MainSigner::None).unwrap(),
            tari_bor::encode(&false).unwrap()
        );
        assert_eq!(
            tari_bor::encode(&MainSigner::SealSigner).unwrap(),
            tari_bor::encode(&true).unwrap()
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn json_round_trips_and_accepts_the_boolean_flag() {
        let public_key = RistrettoPublicKeyBytes::from_bytes(&[7u8; 32]).unwrap();
        for value in [
            MainSigner::None,
            MainSigner::SealSigner,
            MainSigner::Authorizer(public_key),
        ] {
            let json = serde_json::to_string(&value).unwrap();
            assert_eq!(serde_json::from_str::<MainSigner>(&json).unwrap(), value);
        }
        assert_eq!(
            serde_json::from_str::<MainSigner>("true").unwrap(),
            MainSigner::SealSigner
        );
        assert_eq!(serde_json::from_str::<MainSigner>("false").unwrap(), MainSigner::None);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn unsigned_transaction_json_accepts_the_boolean_flag() {
        let unsigned =
            crate::UnsignedTransactionV1::new(1u8, vec![], vec![], Default::default(), None, crate::Epoch(1), false);
        let mut json = serde_json::to_value(&unsigned).unwrap();
        let fields = json.as_object_mut().unwrap();
        assert_eq!(fields.remove("main_signer"), Some(serde_json::json!("SealSigner")));
        fields.insert("is_seal_signer_authorized".to_string(), serde_json::json!(false));

        let decoded: crate::UnsignedTransactionV1 = serde_json::from_value(json).unwrap();
        assert_eq!(decoded.main_signer, MainSigner::None);
    }
}
