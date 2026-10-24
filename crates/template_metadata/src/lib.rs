//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Template metadata types, serialization, and hashing for Tari Ootle templates.
//!
//! This crate provides:
//! - [`TemplateMetadata`] — the standard off-chain metadata structure
//! - [`MetadataHash`] — a multihash of the CBOR-encoded metadata, which converts into the opaque `TemplateMetadataHash`
//!   that the protocol carries
//! - Cargo.toml parsing to extract metadata fields
//! - CBOR and JSON serialization/deserialization

mod cargo_toml;
mod hash;
mod metadata;

pub use cargo_toml::{CargoTomlError, from_cargo_toml, from_cargo_toml_str};
pub use hash::{MetadataHash, MetadataHashError, MetadataHashWriter};
pub use metadata::{FunctionDoc, SCHEMA_VERSION, TemplateMetadata, TemplateMetadataError};
