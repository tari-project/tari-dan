//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Native implementations of the primitives templates reach through
//! `tari_template_lib::intrinsics`.
//!
//! Every intrinsic here is a pure function of its arguments. It cannot touch substate, the
//! workspace, the fee state or the caller — the only thing it receives is [`EngineArgs`] and the
//! only thing it returns is a value. That restriction is what makes two things possible: pricing a
//! call from its declared arguments *before* running it, and reproducing it identically on every
//! validator.
//!
//! # Adding one
//!
//! Give it a fresh [`IntrinsicId`] — never reuse a retired number — and add an arm to both [`price`]
//! and [`dispatch`]. Both must handle it: an id priced but not dispatched charges for work never
//! done, and one dispatched but not priced runs unmetered native code, which is the hole the whole
//! pricing discipline exists to close. `every_id_is_priced_and_dispatched` fails the build if either
//! arm is missed.
//!
//! A new id is a coordinated validator and indexer upgrade. Validators that do not know it answer
//! [`RuntimeError::IntrinsicNotSupported`], naming the id, so an operator sees what is missing
//! rather than a decode failure.

use digest::Digest;
use tari_crypto::{
    keys::{PublicKey as PublicKeyTrait, SecretKey as SecretKeyTrait},
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    tari_utilities::ByteArray,
};
use tari_engine_types::limits::NativeExecutionPoints;
use tari_ootle_common_types::GetVerifier;
use tari_template_lib::{
    args::InvokeResult,
    types::{
        Hash32,
        Hash64,
        bytes::Bytes,
        crypto::{RistrettoPublicKeyBytes, Scalar32Bytes},
        engine_args::{IntrinsicId, SignatureVerifyArg},
    },
};

use crate::runtime::{EngineArgs, RuntimeError};

/// The fewest bytes a [`RistrettoPublicKeyBytes`] can occupy inside an encoded list: 32 bytes of key and the
/// CBOR byte-string header in front of them. `encoded_point_len_is_at_least_the_minimum` pins it.
const MIN_ENCODED_POINT_LEN: u64 = 33;

/// The metering points an intrinsic call costs, derived from its declared arguments alone.
///
/// Charged before [`dispatch`] runs, so the price may not depend on anything the work itself
/// discovers. Where a cost scales with input size — hashing, multi-scalar multiplication — it is
/// read from the argument count or byte length, both known up front.
pub fn price(intrinsic: IntrinsicId, args: &EngineArgs) -> Result<u64, RuntimeError> {
    use IntrinsicId as I;
    let points = match intrinsic {
        I::RISTRETTO_ADD |
        I::RISTRETTO_SUB |
        I::RISTRETTO_NEGATE |
        I::RISTRETTO_IS_IDENTITY |
        I::RISTRETTO_IS_CANONICAL => NativeExecutionPoints::PER_RISTRETTO_OP,
        I::RISTRETTO_MUL => NativeExecutionPoints::PER_RISTRETTO_MUL,
        I::RISTRETTO_MUL_BASE => NativeExecutionPoints::PER_RISTRETTO_MUL_BASE,
        I::RISTRETTO_MSM => {
            // Priced off the encoded length of the point list rather than a decoded one: pricing must not do
            // work proportional to the input it is pricing, and that decode is unmetered. Dividing by the
            // smallest a point can encode to bounds the term count from above, so the charge is never short. A
            // mismatch against the scalar count is rejected in `dispatch`, after this charge — a caller cannot
            // get free work out of an argument this never looks at.
            let terms = args.arg_encoded_len(0) / MIN_ENCODED_POINT_LEN;
            NativeExecutionPoints::PER_RISTRETTO_MSM
                .saturating_add(NativeExecutionPoints::PER_RISTRETTO_MSM_TERM.saturating_mul(terms))
        },
        I::SCALAR_ADD |
        I::SCALAR_SUB |
        I::SCALAR_MUL |
        I::SCALAR_NEGATE |
        I::SCALAR_INVERT |
        I::SCALAR_FROM_UNIFORM_BYTES |
        I::SCALAR_IS_CANONICAL => NativeExecutionPoints::PER_SCALAR_OP,
        I::HASH_BLAKE2B | I::HASH_SHA256 | I::HASH_SHA512 | I::HASH_KECCAK256 => {
            NativeExecutionPoints::PER_HASH.saturating_add(per_byte(args))
        },
        // A verification hashes the domain and message before the group operations, so its cost is
        // not flat in the way the fixed constant alone would suggest.
        I::SCHNORR_VERIFY => NativeExecutionPoints::PER_SCHNORR_VERIFY.saturating_add(per_byte(args)),
        unknown => return Err(RuntimeError::IntrinsicNotSupported { intrinsic: unknown }),
    };
    Ok(points)
}

/// Runs an intrinsic. The caller has already charged [`price`] for it.
pub fn dispatch(intrinsic: IntrinsicId, args: EngineArgs) -> Result<InvokeResult, RuntimeError> {
    use IntrinsicId as I;
    match intrinsic {
        I::RISTRETTO_ADD |
        I::RISTRETTO_SUB |
        I::RISTRETTO_NEGATE |
        I::RISTRETTO_MUL |
        I::RISTRETTO_MUL_BASE |
        I::RISTRETTO_MSM |
        I::RISTRETTO_IS_IDENTITY |
        I::RISTRETTO_IS_CANONICAL => dispatch_ristretto(intrinsic, &args),
        I::SCALAR_ADD |
        I::SCALAR_SUB |
        I::SCALAR_MUL |
        I::SCALAR_NEGATE |
        I::SCALAR_INVERT |
        I::SCALAR_FROM_UNIFORM_BYTES |
        I::SCALAR_IS_CANONICAL => dispatch_scalar(intrinsic, &args),
        I::HASH_BLAKE2B | I::HASH_SHA256 | I::HASH_SHA512 | I::HASH_KECCAK256 => dispatch_hash(intrinsic, &args),
        I::SCHNORR_VERIFY => {
            let SignatureVerifyArg {
                public_key,
                domain,
                message,
                payload,
            } = args.assert_one_arg()?;
            let is_valid = payload.get_verifier().verify(&domain, &message, &public_key, &payload);
            Ok(InvokeResult::encode(&is_valid)?)
        },
        unknown => Err(RuntimeError::IntrinsicNotSupported { intrinsic: unknown }),
    }
}

fn dispatch_ristretto(intrinsic: IntrinsicId, args: &EngineArgs) -> Result<InvokeResult, RuntimeError> {
    use IntrinsicId as I;
    match intrinsic {
        I::RISTRETTO_ADD => {
            let (a, b) = two_points(args)?;
            encode_point(&a + &b)
        },
        I::RISTRETTO_SUB => {
            let (a, b) = two_points(args)?;
            encode_point(&a - &b)
        },
        I::RISTRETTO_NEGATE => {
            let a = point(args, 0)?;
            encode_point(&RistrettoPublicKey::default() - &a)
        },
        I::RISTRETTO_MUL => {
            let p = point(args, 0)?;
            let s = scalar(args, 1)?;
            encode_point(&p * &s)
        },
        I::RISTRETTO_MUL_BASE => {
            let s = scalar(args, 0)?;
            encode_point(RistrettoPublicKey::from_secret_key(&s))
        },
        I::RISTRETTO_MSM => {
            let points = args.get::<Vec<RistrettoPublicKeyBytes>>(0)?;
            let scalars = args.get::<Vec<Scalar32Bytes>>(1)?;
            if points.len() != scalars.len() {
                return Err(RuntimeError::InvalidArgument {
                    argument: "ristretto_msm",
                    reason: format!(
                        "one scalar per point required, got {} points and {} scalars",
                        points.len(),
                        scalars.len()
                    ),
                });
            }
            let points = points.iter().map(decode_point).collect::<Result<Vec<_>, _>>()?;
            let scalars = scalars.iter().map(decode_scalar).collect::<Result<Vec<_>, _>>()?;
            encode_point(RistrettoPublicKey::batch_mul(&scalars, &points))
        },
        I::RISTRETTO_IS_IDENTITY => {
            let a = point(args, 0)?;
            Ok(InvokeResult::encode(&(a == RistrettoPublicKey::default()))?)
        },
        I::RISTRETTO_IS_CANONICAL => {
            let bytes = args.get::<RistrettoPublicKeyBytes>(0)?;
            Ok(InvokeResult::encode(&decode_point(&bytes).is_ok())?)
        },
        unknown => Err(RuntimeError::IntrinsicNotSupported { intrinsic: unknown }),
    }
}

fn dispatch_scalar(intrinsic: IntrinsicId, args: &EngineArgs) -> Result<InvokeResult, RuntimeError> {
    use IntrinsicId as I;
    match intrinsic {
        I::SCALAR_ADD => {
            let (a, b) = two_scalars(args)?;
            encode_scalar(&a + &b)
        },
        I::SCALAR_SUB => {
            let (a, b) = two_scalars(args)?;
            encode_scalar(&a - &b)
        },
        I::SCALAR_MUL => {
            let (a, b) = two_scalars(args)?;
            encode_scalar(&a * &b)
        },
        I::SCALAR_NEGATE => {
            let a = scalar(args, 0)?;
            encode_scalar(&RistrettoSecretKey::default() - &a)
        },
        I::SCALAR_INVERT => {
            let inverse = scalar(args, 0)?
                .invert()
                .map(|s| Scalar32Bytes::from_bytes(s.as_bytes()))
                .transpose()
                .map_err(|e| RuntimeError::InvalidArgument {
                    argument: "scalar_invert",
                    reason: e.to_string(),
                })?;
            Ok(InvokeResult::encode(&inverse)?)
        },
        I::SCALAR_FROM_UNIFORM_BYTES => {
            let bytes = args.get::<Bytes>(0)?;
            let s = RistrettoSecretKey::from_uniform_bytes(&bytes).map_err(|e| RuntimeError::InvalidArgument {
                argument: "scalar_from_uniform_bytes",
                reason: format!("expected 64 uniform bytes: {e}"),
            })?;
            encode_scalar(s)
        },
        I::SCALAR_IS_CANONICAL => {
            let bytes = args.get::<Scalar32Bytes>(0)?;
            Ok(InvokeResult::encode(&decode_scalar(&bytes).is_ok())?)
        },
        unknown => Err(RuntimeError::IntrinsicNotSupported { intrinsic: unknown }),
    }
}

fn dispatch_hash(intrinsic: IntrinsicId, args: &EngineArgs) -> Result<InvokeResult, RuntimeError> {
    use IntrinsicId as I;
    let parts = hash_parts(args)?;
    macro_rules! digest_32 {
        ($hasher:ty) => {{
            let mut hasher = <$hasher>::default();
            for part in &parts {
                hasher.update(part);
            }
            Ok(InvokeResult::encode(&Hash32::from(<[u8; 32]>::from(
                hasher.finalize(),
            )))?)
        }};
    }
    match intrinsic {
        I::HASH_BLAKE2B => digest_32!(blake2::Blake2b<digest::consts::U32>),
        I::HASH_SHA256 => digest_32!(sha2::Sha256),
        I::HASH_KECCAK256 => digest_32!(sha3::Keccak256),
        I::HASH_SHA512 => {
            let mut hasher = sha2::Sha512::default();
            for part in &parts {
                hasher.update(part);
            }
            Ok(InvokeResult::encode(&Hash64::from(<[u8; 64]>::from(
                hasher.finalize(),
            )))?)
        },
        unknown => Err(RuntimeError::IntrinsicNotSupported { intrinsic: unknown }),
    }
}

/// The byte-length charge for an intrinsic whose cost scales with how much data it is given.
///
/// Measured on the encoded arguments rather than the decoded ones: the length is then known without
/// decoding, so a blob is not copied a second time merely to be charged for, and the few bytes of
/// CBOR framing per argument make the charge conservative rather than short.
fn per_byte(args: &EngineArgs) -> u64 {
    NativeExecutionPoints::PER_HASH_BYTE.saturating_mul(args.encoded_len())
}

/// A hash intrinsic takes one list of parts and digests their concatenation, so a caller walking a
/// Merkle path pays one engine crossing per level rather than one per node. Hashing a single blob is
/// the one-element case, which keeps the wire shape unambiguous.
fn hash_parts(args: &EngineArgs) -> Result<Vec<Bytes>, RuntimeError> {
    args.get::<Vec<Bytes>>(0)
}

fn point(args: &EngineArgs, index: usize) -> Result<RistrettoPublicKey, RuntimeError> {
    decode_point(&args.get::<RistrettoPublicKeyBytes>(index)?)
}

fn scalar(args: &EngineArgs, index: usize) -> Result<RistrettoSecretKey, RuntimeError> {
    decode_scalar(&args.get::<Scalar32Bytes>(index)?)
}

fn two_points(args: &EngineArgs) -> Result<(RistrettoPublicKey, RistrettoPublicKey), RuntimeError> {
    Ok((point(args, 0)?, point(args, 1)?))
}

fn two_scalars(args: &EngineArgs) -> Result<(RistrettoSecretKey, RistrettoSecretKey), RuntimeError> {
    Ok((scalar(args, 0)?, scalar(args, 1)?))
}

fn decode_point(bytes: &RistrettoPublicKeyBytes) -> Result<RistrettoPublicKey, RuntimeError> {
    RistrettoPublicKey::from_canonical_bytes(&**bytes).map_err(|e| RuntimeError::InvalidArgument {
        argument: "ristretto point",
        reason: format!("not a canonical Ristretto point: {e}"),
    })
}

fn decode_scalar(bytes: &Scalar32Bytes) -> Result<RistrettoSecretKey, RuntimeError> {
    RistrettoSecretKey::from_canonical_bytes(&**bytes).map_err(|e| RuntimeError::InvalidArgument {
        argument: "scalar",
        reason: format!("not a canonically reduced scalar: {e}"),
    })
}

fn encode_point(p: RistrettoPublicKey) -> Result<InvokeResult, RuntimeError> {
    let bytes = RistrettoPublicKeyBytes::from_bytes(p.as_bytes()).map_err(|e| RuntimeError::InvalidArgument {
        argument: "ristretto point",
        reason: e.to_string(),
    })?;
    Ok(InvokeResult::encode(&bytes)?)
}

fn encode_scalar(s: RistrettoSecretKey) -> Result<InvokeResult, RuntimeError> {
    let bytes = Scalar32Bytes::from_bytes(s.as_bytes()).map_err(|e| RuntimeError::InvalidArgument {
        argument: "scalar",
        reason: e.to_string(),
    })?;
    Ok(InvokeResult::encode(&bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `price` bounds an MSM's term count by dividing the encoded argument by [`MIN_ENCODED_POINT_LEN`], which
    /// only bounds from above while a point never encodes to less than that.
    #[test]
    fn encoded_point_len_is_at_least_the_minimum() {
        for len in [1usize, 2, 32, 1000] {
            let points = vec![RistrettoPublicKeyBytes::zero(); len];
            let encoded = tari_bor::encode(&points).unwrap().len() as u64;
            assert!(
                encoded >= MIN_ENCODED_POINT_LEN * len as u64,
                "{len} points encode to {encoded} bytes, under the {MIN_ENCODED_POINT_LEN} per point assumed when \
                 pricing"
            );
        }
    }

    /// An id priced but not dispatched charges for work never done; one dispatched but not priced
    /// runs unmetered native code, which is the hole the pricing discipline exists to close. Empty
    /// arguments make both fail, but only a *missing arm* answers `IntrinsicNotSupported` — which is
    /// what this distinguishes.
    #[test]
    fn every_id_is_priced_and_dispatched() {
        for &id in IntrinsicId::ALL {
            assert!(
                !matches!(
                    price(id, &EngineArgs::new()),
                    Err(RuntimeError::IntrinsicNotSupported { .. })
                ),
                "{id} has no arm in `price`",
            );
            assert!(
                !matches!(
                    dispatch(id, EngineArgs::new()),
                    Err(RuntimeError::IntrinsicNotSupported { .. })
                ),
                "{id} has no arm in `dispatch`",
            );
        }
    }

    /// An unknown id is reported as unsupported rather than as a malformed argument, so an operator
    /// running a validator behind the template sees that an upgrade is what is missing.
    #[test]
    fn an_unknown_id_reports_as_unsupported() {
        let unknown = IntrinsicId(0xDEAD_BEEF);
        assert!(matches!(
            price(unknown, &EngineArgs::new()),
            Err(RuntimeError::IntrinsicNotSupported { .. })
        ));
        assert!(matches!(
            dispatch(unknown, EngineArgs::new()),
            Err(RuntimeError::IntrinsicNotSupported { .. })
        ));
    }
}
