//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

#[macro_export]
macro_rules! create_hash_type {
    (
         $(#[$meta:meta])*
        $name:ident) => {
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            ::serde::Serialize,
            ::serde::Deserialize,
            ::borsh::BorshSerialize,
        )]
        #[serde(transparent)]
        $(#[$meta])*
        pub struct $name(#[serde(with = "::ootle_serde::hex")] ::tari_sidechain::FixedHash);

        // Minicbor codec: encode as a CBOR byte string of the fixed-size hash, mirroring the
        // serde `transparent` representation. FixedHash is a foreign type so derives on the
        // wrapping struct are not possible — implement directly on the newtype.
        impl<C> ::minicbor::Encode<C> for $name {
            fn encode<W: ::minicbor::encode::Write>(
                &self,
                e: &mut ::minicbor::Encoder<W>,
                _ctx: &mut C,
            ) -> Result<(), ::minicbor::encode::Error<W::Error>> {
                e.bytes(self.0.as_slice())?;
                Ok(())
            }
        }

        impl<'b, C> ::minicbor::Decode<'b, C> for $name {
            fn decode(
                d: &mut ::minicbor::Decoder<'b>,
                _ctx: &mut C,
            ) -> Result<Self, ::minicbor::decode::Error> {
                let bytes = d.bytes()?;
                let hash = ::tari_sidechain::FixedHash::try_from(bytes)
                    .map_err(|e| ::minicbor::decode::Error::message(::std::format!(
                        concat!(stringify!($name), " decode failed: {}"), e
                    )))?;
                Ok(Self(hash))
            }
        }

        impl<C> ::minicbor::CborLen<C> for $name {
            fn cbor_len(&self, ctx: &mut C) -> usize {
                let n = ::tari_sidechain::FixedHash::byte_size();
                <u64 as ::minicbor::CborLen<C>>::cbor_len(&(n as u64), ctx) + n
            }
        }

        impl $name {
            /// Represents a zero/null hash.
            pub const fn zero() -> Self {
                Self(::tari_sidechain::FixedHash::zero())
            }

            pub fn new<T: Into<::tari_sidechain::FixedHash>>(hash: T) -> Self {
                Self(hash.into())
            }

            pub const fn hash(&self) -> &::tari_sidechain::FixedHash {
                &self.0
            }

            pub fn as_bytes(&self) -> &[u8] {
                self.0.as_slice()
            }

            pub fn is_zero(&self) -> bool {
                self.0.iter().all(|b| *b == 0)
            }

            pub const fn into_array(self) -> [u8; 32] {
                self.0.into_array()
            }

            pub const fn byte_size() -> usize {
                ::tari_sidechain::FixedHash::byte_size()
            }
        }

        impl AsRef<[u8]> for $name {
            fn as_ref(&self) -> &[u8] {
                self.0.as_slice()
            }
        }

        impl From<::tari_sidechain::FixedHash> for $name {
            fn from(value: ::tari_sidechain::FixedHash) -> Self {
                Self(value)
            }
        }

        impl From<[u8; ::tari_sidechain::FixedHash::byte_size()]> for $name {
            fn from(value: [u8; ::tari_sidechain::FixedHash::byte_size()]) -> Self {
                Self(value.into())
            }
        }

        impl TryFrom<Vec<u8>> for $name {
            type Error = tari_sidechain::FixedHashSizeError;

            fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
                value.as_slice().try_into()
            }
        }

        impl TryFrom<&[u8]> for $name {
            type Error = tari_sidechain::FixedHashSizeError;

            fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
                ::tari_sidechain::FixedHash::try_from(value).map(Self)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Display::fmt(&self.0, f)
            }
        }

        impl AsRef<$name> for $name {
            fn as_ref(&self) -> &$name {
                self
            }
        }
    };
}
