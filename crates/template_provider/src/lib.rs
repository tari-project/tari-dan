//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Shared template provider chain for Tari Ootle.
//!
//! [`MemoryCacheTemplateProvider`] is the outermost layer: an in-memory cache of compiled templates with
//! builtins precached and concurrent first-fetches coalesced per address (so a cache miss never triggers
//! more than one load of the same template). It delegates to an inner [`TemplateProvider`] on miss — the
//! validator node wraps a disk-cache + raw state store, the indexer wraps an on-demand network fetcher.
//!
//! [`UncommittedTemplateCache`] holds compiled templates published in uncommitted blocks, outside that chain.
//!
//! [`TemplateProvider`]: tari_ootle_common_types::services::template_provider::TemplateProvider

mod cmap_semaphore;

mod memory_cache;
pub use memory_cache::{
    MemoryCacheTemplateProvider,
    MemoryCacheTemplateProviderError,
    ResidentTemplateProvider,
    TemplateConfig,
    builtin_resident_bytes,
};

mod uncommitted_cache;
pub use uncommitted_cache::UncommittedTemplateCache;
