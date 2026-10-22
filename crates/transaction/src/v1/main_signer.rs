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
/// tag is the same byte), and `Authorizer` as the key. JSON uses the externally tagged form: `"None"`,
/// `"SealSigner"` or `{"Authorizer": <key>}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, borsh::BorshSerialize)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
    fn json_round_trips_in_the_tagged_form() {
        let public_key = RistrettoPublicKeyBytes::from_bytes(&[7u8; 32]).unwrap();
        for value in [
            MainSigner::None,
            MainSigner::SealSigner,
            MainSigner::Authorizer(public_key),
        ] {
            let json = serde_json::to_string(&value).unwrap();
            assert_eq!(serde_json::from_str::<MainSigner>(&json).unwrap(), value);
        }
        assert!(serde_json::from_str::<MainSigner>("true").is_err());
    }
}
