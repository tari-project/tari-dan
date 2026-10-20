//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, sync::Arc};

use log::debug;
use serde::{Deserialize, Serialize};
use tari_engine::{template::LoadedTemplate, wasm::WasmModule};
use tari_ootle_common_types::services::template_provider::{
    TemplateMetadataProvider,
    TemplateProvider,
    TemplateProviderMetadata,
};
use tari_template_builtin::all_builtin_templates;
use tari_template_lib_types::TemplateAddress;

use crate::cmap_semaphore;

const LOG_TARGET: &str = "tari::ootle::template_provider::memory_cache";
pub(crate) const CONCURRENT_ACCESS_LIMIT: isize = 100;

/// Multiplier applied to the WASM source byte count when weighing cache
/// entries. The source size under-estimates the resident footprint of a
/// `LoadedTemplate` because the dominant cost is the Cranelift-compiled
/// artifact (machine code + relocations + frame info), which inflates the
/// source by ~1.5–3× in typical templates and up to ~5× for hot-loop heavy
/// code. K=4 keeps the bound honest in the typical case and overshoots by at
/// most ~25% in the worst case — acceptable for a coarse process-memory
/// budget. Cheaper than serializing each module on insert and infallible.
pub(crate) const CODE_SIZE_TO_RESIDENT_BYTES_FACTOR: usize = 4;

/// Resident cost of the builtin templates, which are held for the life of a
/// [`MemoryCacheTemplateProvider`] rather than cached under [`TemplateConfig::max_cache_size_bytes`].
///
/// Weighed like a cache entry so the two are directly comparable.
pub fn builtin_resident_bytes() -> u64 {
    all_builtin_templates()
        .iter()
        .map(|t| (t.binary.len() * CODE_SIZE_TO_RESIDENT_BYTES_FACTOR) as u64)
        .sum()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TemplateConfig {
    #[serde(default = "default_max_cache_size_bytes")]
    max_cache_size_bytes: u64,
    #[serde(default = "default_max_disk_cache_size_bytes")]
    max_disk_cache_size_bytes: u64,
}

/// Weighed at [`CODE_SIZE_TO_RESIDENT_BYTES_FACTOR`] times each template's source, 1 GiB holds
/// roughly 870 templates of the 300 KiB average the disk cap is sized against — more than a busy
/// shard group's working set, so the process rarely reaches the disk tier at all.
fn default_max_cache_size_bytes() -> u64 {
    1024 * 1024 * 1024
}

/// A compiled artifact runs about ten times the size of its WASM source, so a node that has served
/// a few thousand templates would otherwise hold tens of GiB of them.
///
/// At that expansion and a 300 KiB average source, 10 GiB holds roughly 3,500 artifacts. That is
/// sized to outlast the templates a node actually calls rather than every template it has ever
/// seen: past the cap the coldest artifacts are deleted and recompiled if they are wanted again, so
/// the cost of the bound being too low is CPU, and the cost of it being too high is disk an
/// operator may want elsewhere.
fn default_max_disk_cache_size_bytes() -> u64 {
    10 * 1024 * 1024 * 1024
}

impl Default for TemplateConfig {
    fn default() -> Self {
        Self {
            max_cache_size_bytes: default_max_cache_size_bytes(),
            max_disk_cache_size_bytes: default_max_disk_cache_size_bytes(),
        }
    }
}

impl TemplateConfig {
    /// Bound on the in-memory cache of compiled modules. Builtins are held outside it, and cost a
    /// further [`builtin_resident_bytes`].
    pub fn max_cache_size_bytes(&self) -> u64 {
        self.max_cache_size_bytes
    }

    /// Bound on the on-disk cache of serialized compiled modules.
    pub fn max_disk_cache_size_bytes(&self) -> u64 {
        self.max_disk_cache_size_bytes
    }
}

/// A provider that can say whether a template is already compiled and held in memory.
///
/// Answered from memory alone: a `false` is "not resident", which is distinct from "not known" —
/// the template may still be on disk or in the state store. Callers that want the template itself
/// go through [`TemplateProvider::get_template`].
pub trait ResidentTemplateProvider {
    fn is_resident(&self, address: &TemplateAddress) -> bool;
}

/// Outermost layer of the template provider chain.
///
/// Holds an in-memory moka cache of `LoadedTemplate`s keyed by address and a
/// per-address semaphore to coalesce concurrent first-fetches. Builtin
/// templates are precompiled into the cache at construction time. Everything
/// else delegates to `inner` on miss. The validator node layers it as:
///
/// ```text
/// MemoryCacheTemplateProvider          (this)
///   └── DiskCachedWasmTemplateProvider (compiled-module disk cache + compile)
///         └── ValidatorNodeStateStore  (raw PublishedTemplate bytes from rocksdb)
/// ```
///
/// while the indexer layers it as:
///
/// ```text
/// MemoryCacheTemplateProvider          (this)
///   └── LazyTemplateProvider           (on-demand fetch: disk cache → local store → network)
/// ```
#[derive(Clone)]
pub struct MemoryCacheTemplateProvider<TInner> {
    inner: TInner,
    builtins: Arc<HashMap<TemplateAddress, LoadedTemplate>>,
    cache: mini_moka::sync::Cache<TemplateAddress, LoadedTemplate>,
    cmap_semaphore: cmap_semaphore::ConcurrentMapSemaphore<TemplateAddress>,
}

impl<TInner> MemoryCacheTemplateProvider<TInner>
where TInner: TemplateProvider<Template = LoadedTemplate>
{
    pub fn new(inner: TInner, config: &TemplateConfig) -> Self {
        let cache = mini_moka::sync::Cache::builder()
            .weigher(|_, t: &LoadedTemplate| {
                let est = t.code_size().saturating_mul(CODE_SIZE_TO_RESIDENT_BYTES_FACTOR);
                u32::try_from(est).unwrap_or(u32::MAX)
            })
            .max_capacity(config.max_cache_size_bytes())
            .build();

        // Builtins are held apart from the evictable cache and for the life of the provider.
        // Compile directly here — builtins live only in memory, never go through the disk-cache
        // layer (their addresses are hardcoded constants and would otherwise pin stale compiled
        // modules across builtin recompiles). The disk-cache layer also has a matching bypass on
        // the lookup side as defence in depth. That bypass is what makes residency here
        // load-bearing: an evicted builtin has no artifact to deserialize and recompiles from
        // source on the caller's thread, and every account transaction calls one.
        let builtins = all_builtin_templates()
            .iter()
            .map(|template| {
                (
                    template.address,
                    WasmModule::load_template_from_code(template.binary).expect("Built-in template failed to load"),
                )
            })
            .collect();

        Self {
            inner,
            builtins: Arc::new(builtins),
            cache,
            cmap_semaphore: cmap_semaphore::ConcurrentMapSemaphore::new(CONCURRENT_ACCESS_LIMIT),
        }
    }
}

impl<TInner> ResidentTemplateProvider for MemoryCacheTemplateProvider<TInner> {
    fn is_resident(&self, address: &TemplateAddress) -> bool {
        self.builtins.contains_key(address) || self.cache.contains_key(address)
    }
}

impl<TInner> TemplateProvider for MemoryCacheTemplateProvider<TInner>
where TInner: TemplateProvider<Template = LoadedTemplate> + Clone + 'static
{
    type Error = MemoryCacheTemplateProviderError;
    type Template = LoadedTemplate;

    fn get_template(&self, address: &TemplateAddress) -> Result<Option<Self::Template>, Self::Error> {
        if let Some(template) = self.builtins.get(address) {
            return Ok(Some(template.clone()));
        }
        if let Some(template) = self.cache.get(address) {
            debug!(target: LOG_TARGET, "CACHE HIT: Template {}", address);
            return Ok(Some(template));
        }
        debug!(target: LOG_TARGET, "CACHE MISS: Template {}", address);

        // This protects the following critical area by:
        // 1. preventing more than CONCURRENT_ACCESS_LIMIT concurrent accesses
        // 2. preventing more than one load of the same template
        // The reasons are:
        // 1. for efficiency, to only ever load the template once (until it is purged from the cache), and
        // 2. to prevent stack overflow. This happens in stress testing, if around 200 templates are loaded concurrently
        let guard = self.cmap_semaphore.acquire(*address);
        let _access = guard.access();

        // After acquiring the semaphore, the racing thread may have populated
        // the cache; check again before delegating to the inner provider.
        if let Some(template) = self.cache.get(address) {
            return Ok(Some(template));
        }

        let Some(loaded) = self
            .inner
            .get_template(address)
            .map_err(|e| MemoryCacheTemplateProviderError::Inner(e.into()))?
        else {
            return Ok(None);
        };

        self.cache.insert(*address, loaded.clone());
        Ok(Some(loaded))
    }

    fn has_template(&self, id: &TemplateAddress) -> Result<bool, Self::Error> {
        Ok(self.builtins.contains_key(id) ||
            self.cache.contains_key(id) ||
            self.inner
                .has_template(id)
                .map_err(|e| MemoryCacheTemplateProviderError::Inner(e.into()))?)
    }
}

impl<TInner> TemplateMetadataProvider for MemoryCacheTemplateProvider<TInner>
where TInner: TemplateProvider<Template = LoadedTemplate> + TemplateMetadataProvider + Clone + 'static
{
    fn get_template_metadata(&self, id: &TemplateAddress) -> Result<Option<TemplateProviderMetadata>, Self::Error> {
        // The hot cache only stores compiled modules, not metadata fields.
        // Always delegate.
        self.inner
            .get_template_metadata(id)
            .map_err(|e| MemoryCacheTemplateProviderError::Inner(e.into()))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MemoryCacheTemplateProviderError {
    #[error(transparent)]
    Inner(anyhow::Error),
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;

    use super::*;

    #[derive(Clone, Default)]
    struct CountingProvider {
        calls: Arc<AtomicUsize>,
    }

    #[derive(Debug, thiserror::Error)]
    #[error("no template")]
    struct NoTemplate;

    impl TemplateProvider for CountingProvider {
        type Error = NoTemplate;
        type Template = LoadedTemplate;

        fn get_template(&self, _address: &TemplateAddress) -> Result<Option<Self::Template>, Self::Error> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(None)
        }
    }

    #[test]
    fn builtins_are_served_under_a_cache_budget_that_holds_nothing() {
        let inner = CountingProvider::default();
        let calls = inner.calls.clone();
        let provider = MemoryCacheTemplateProvider::new(inner, &TemplateConfig {
            max_cache_size_bytes: 1,
            max_disk_cache_size_bytes: 1,
        });

        let template = provider
            .get_template(&ACCOUNT_TEMPLATE_ADDRESS)
            .unwrap()
            .expect("account builtin");
        assert_eq!(template.template_name(), "Account");
        assert!(provider.has_template(&ACCOUNT_TEMPLATE_ADDRESS).unwrap());
        assert_eq!(
            calls.load(Ordering::Relaxed),
            0,
            "a builtin must not reach the inner provider: it has no disk tier to fall back to",
        );
    }
}
