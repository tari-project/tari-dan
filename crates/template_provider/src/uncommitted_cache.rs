//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine::{
    template::{LoadedTemplate, TemplateLoaderError},
    wasm::WasmModule,
};
use tari_template_lib_types::TemplateAddress;

use crate::{
    cmap_semaphore::ConcurrentMapSemaphore,
    memory_cache::{CODE_SIZE_TO_RESIDENT_BYTES_FACTOR, CONCURRENT_ACCESS_LIMIT},
};

/// Compiled templates published in uncommitted blocks.
///
/// Held apart from the [`MemoryCacheTemplateProvider`](crate::MemoryCacheTemplateProvider) chain, whose layers serve
/// a hit without asking whether the template exists and so may only ever hold what committed state served. A
/// template published in a block that is later orphaned exists on no chain, so a caller loads from here only once
/// the chain it executes on is known to publish the address. A template address commits to its binary, so an entry
/// is right for every chain that publishes the address.
#[derive(Clone)]
pub struct UncommittedTemplateCache {
    cache: mini_moka::sync::Cache<TemplateAddress, LoadedTemplate>,
    /// Admits one compile per address at a time, so concurrent first loads of a template compile it once.
    cmap_semaphore: ConcurrentMapSemaphore<TemplateAddress>,
}

impl UncommittedTemplateCache {
    pub fn new(max_resident_bytes: u64) -> Self {
        let cache = mini_moka::sync::Cache::builder()
            .weigher(|_, t: &LoadedTemplate| {
                let est = t.code_size().saturating_mul(CODE_SIZE_TO_RESIDENT_BYTES_FACTOR);
                u32::try_from(est).unwrap_or(u32::MAX)
            })
            .max_capacity(max_resident_bytes)
            .build();
        Self {
            cache,
            cmap_semaphore: ConcurrentMapSemaphore::new(CONCURRENT_ACCESS_LIMIT),
        }
    }

    /// The compiled template published at `address` with `binary`, compiling it on a miss.
    pub fn load(&self, address: TemplateAddress, binary: &[u8]) -> Result<LoadedTemplate, TemplateLoaderError> {
        if let Some(loaded) = self.cache.get(&address) {
            return Ok(loaded);
        }
        let guard = self.cmap_semaphore.acquire(address);
        let _access = guard.access();
        // A load that held the address before this one may have filled the cache.
        if let Some(loaded) = self.cache.get(&address) {
            return Ok(loaded);
        }
        let loaded = WasmModule::load_template_from_code(binary)?;
        self.cache.insert(address, loaded.clone());
        Ok(loaded)
    }
}

impl std::fmt::Debug for UncommittedTemplateCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UncommittedTemplateCache")
            .field("entry_count", &self.cache.entry_count())
            .finish()
    }
}
