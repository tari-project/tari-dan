//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! On-disk cache for compiled wasmer modules.
//!
//! Cranelift compilation of WASM templates is expensive: ~6 MB peak heap and
//! tens of milliseconds per template, paid on every node startup. The compiled
//! output for a given `(wasm_source, engine_config)` is deterministic and
//! reusable, so it can be persisted on local disk and loaded back via
//! [`wasmer::Module::deserialize_unchecked`] in milliseconds with negligible
//! peak heap.
//!
//! The WASM source bytes themselves stay on-chain (canonical, deterministic
//! representation). This cache is strictly node-local: any corrupt or missing
//! entry falls back to a full compile from source, with no consensus
//! implication.
//!
//! Two surfaces are exposed:
//!
//! - [`WasmModuleCache`] — low-level helper for callers that don't sit in a `TemplateProvider` chain (e.g. the wallet
//!   daemon's template monitor).
//! - [`DiskCachedWasmTemplateProvider`] — a `TemplateProvider` middleware that wraps a raw `PublishedTemplate` provider
//!   and outputs `LoadedTemplate`, doing compile-or-deserialize behind the scenes.

use std::{
    fs,
    io,
    path::{Path, PathBuf},
};

use log::*;
use memmap2::Mmap;
use tari_engine_types::published_template::PublishedTemplate;
use tari_ootle_common_types::{
    Epoch,
    services::template_provider::{TemplateMetadataProvider, TemplateProvider, TemplateProviderMetadata},
};
use tari_template_builtin::is_builtin_template_address;
use tari_template_lib::types::TemplateAddress;

use crate::{
    template::{LoadedTemplate, TemplateLoaderError},
    wasm::WasmModule,
};

const LOG_TARGET: &str = "tari::engine::wasm::cache";

/// Engine-config fingerprint embedded in cache filenames.
///
/// Bump this string whenever any of the following change, otherwise nodes
/// loading from a stale cache will misbehave (deserialize failures at best,
/// undefined behaviour at worst):
///
/// - [`crate::wasm::WasmModule::create_engine`] config (compiler flags, features bitset, middleware list, tunables).
/// - [`tari_engine_types::limits::MAX_WASM_POINTS_PER_CALL`], which the metering middleware bakes into the artifact as
///   the initial remaining-points global. `WasmProcess::metering_allowance` reads it back with `get_remaining_points`,
///   so a node serving a stale artifact meters against the old cap and diverges from one that compiled fresh.
/// - The `wasmer` crate version (the serialized artifact format is internal to wasmer and not part of any stable wire
///   spec).
///
/// On a bump, old cache files become orphans (different filename suffix)
/// and the next compile-from-source rewrites under the new key.
pub const ENGINE_FINGERPRINT: &str = "v5";

/// 8-byte LE length prefix at the head of each cache file holding the original
/// WASM source byte count. `wasmer::Module::serialize` doesn't preserve this
/// and downstream consumers use it for accounting (e.g. moka weighing).
const HEADER_BYTES: usize = 8;

/// Low-level on-disk cache for compiled wasmer modules.
///
/// Files live at `{dir}/{template_address}_{ENGINE_FINGERPRINT}.bin`.
/// The body is `[u64 LE: code_size] || wasmer::Module::serialize(...)`.
///
/// Writes are atomic (tempfile + rename). Read failures (missing file,
/// deserialize errors, format changes) are non-fatal: the corrupt file is
/// removed and the caller is expected to recompile from source.
#[derive(Debug, Clone)]
pub struct WasmModuleCache {
    dir: PathBuf,
}

impl WasmModuleCache {
    /// Open or create a cache rooted at `dir`. Creates the directory tree
    /// if missing.
    pub fn open(dir: impl Into<PathBuf>) -> io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path_for(&self, addr: &TemplateAddress) -> PathBuf {
        self.dir.join(format!("{}_{}.bin", addr, ENGINE_FINGERPRINT))
    }

    /// Try to load a previously-cached module for `addr`. Returns `None` on
    /// any miss — file missing, header malformed, deserialize failure. On
    /// recoverable corruption the bad file is removed so a subsequent `store`
    /// can replace it.
    ///
    /// The file is `mmap`'d rather than read into a `Vec<u8>` — wasmer's
    /// deserialize path accepts `bytes::Bytes` and `Bytes::from_owner` lets us
    /// hand it the mmap region without copying. Cache hits cost a single
    /// `mmap` syscall (and the page faults wasmer's deserializer triggers as
    /// it walks the artifact); no full-artifact allocation.
    pub fn try_load(&self, addr: &TemplateAddress) -> Option<LoadedTemplate> {
        let path = self.path_for(addr);
        let file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
            Err(e) => {
                warn!(
                    target: LOG_TARGET,
                    "Failed to open cache file {}: {}", path.display(), e,
                );
                return None;
            },
        };

        // SAFETY: see the docs on `Mmap::map`. We don't promise immutability
        // of the underlying file — if another process truncates or rewrites
        // it concurrently the mmap read could SIGBUS. The cache dir is owned
        // by this process (single writer, atomic rename on update), so this
        // is safe in the deployment model. The fingerprint-suffixed filename
        // also means concurrent writers from a different engine config would
        // target a different file.
        let mmap = match unsafe { Mmap::map(&file) } {
            Ok(m) => m,
            Err(e) => {
                warn!(
                    target: LOG_TARGET,
                    "Failed to mmap cache file {}: {}", path.display(), e,
                );
                return None;
            },
        };

        if mmap.len() < HEADER_BYTES {
            warn!(
                target: LOG_TARGET,
                "Cache file {} is shorter than the {}-byte header; removing.",
                path.display(),
                HEADER_BYTES,
            );
            drop(mmap);
            let _ignore = fs::remove_file(&path);
            return None;
        }

        let mut header = [0u8; HEADER_BYTES];
        header.copy_from_slice(&mmap[..HEADER_BYTES]);
        let code_size = u64::from_le_bytes(header) as usize;

        // Wrap the mmap as a Bytes that owns it, then slice past the
        // 8-byte header. `Bytes::slice` is zero-copy (pointer + length
        // adjustment); the wrapped Mmap is dropped only when the resulting
        // Bytes (and any clones the deserializer may keep) goes out of
        // scope.
        let body = bytes::Bytes::from_owner(mmap).slice(HEADER_BYTES..);

        // SAFETY: bytes were written by [`Self::store`] in a previous run of
        // this process (or an earlier process owning the same data dir) via
        // `wasmer::Module::serialize`. The cache directory is node-local and
        // not attacker-controlled in any sane operational setup. The
        // fingerprint suffix in the filename guarantees the engine config
        // matches this build; a deserialize failure simply triggers the
        // recompile fallback.
        match unsafe { WasmModule::load_template_from_serialized(body, code_size) } {
            Ok(loaded) => {
                debug!(target: LOG_TARGET, "Cache hit for template {}", addr);
                Some(loaded)
            },
            Err(err) => {
                warn!(
                    target: LOG_TARGET,
                    "Failed to deserialize cached module {}: {}; removing.",
                    path.display(),
                    err,
                );
                let _ignore = fs::remove_file(&path);
                None
            },
        }
    }

    /// Persist a compiled module under `addr`. Best-effort: on any failure
    /// (serialize, write, rename) a warning is logged and the call returns
    /// successfully — the caller's compiled module is still valid.
    pub fn store(&self, addr: &TemplateAddress, loaded: &LoadedTemplate) {
        let LoadedTemplate::Wasm(wasm) = loaded;
        let serialized = match wasm.wasm_module().serialize() {
            Ok(s) => s,
            Err(e) => {
                warn!(target: LOG_TARGET, "Failed to serialize module for {}: {}", addr, e);
                return;
            },
        };

        let path = self.path_for(addr);
        let tmp = self.dir.join(format!(
            "{}_{}.bin.tmp.{}",
            addr,
            ENGINE_FINGERPRINT,
            std::process::id(),
        ));

        let mut bytes = Vec::with_capacity(HEADER_BYTES + serialized.len());
        bytes.extend_from_slice(&(wasm.code_size() as u64).to_le_bytes());
        bytes.extend_from_slice(&serialized);

        if let Err(e) = fs::write(&tmp, &bytes) {
            warn!(target: LOG_TARGET, "Failed to write cache tempfile {}: {}", tmp.display(), e);
            return;
        }

        if let Err(e) = fs::rename(&tmp, &path) {
            warn!(
                target: LOG_TARGET,
                "Failed to rename {} -> {}: {}", tmp.display(), path.display(), e,
            );
            let _ignore = fs::remove_file(&tmp);
            return;
        }

        debug!(
            target: LOG_TARGET,
            "Cached compiled module for template {} -> {}", addr, path.display(),
        );
    }
}

/// `TemplateProvider` middleware that adds an on-disk compiled-module cache
/// behind any provider returning raw [`PublishedTemplate`] bytes.
///
/// On `get_template(addr)`:
/// 1. If the cache file `{addr}_{ENGINE_FINGERPRINT}.bin` exists, deserialize and return — no compile, no
///    inner-provider call.
/// 2. Otherwise delegate to `inner` for the raw `PublishedTemplate`, compile via
///    [`WasmModule::load_template_from_code`], persist the compiled module to the cache, return.
///
/// Intended placement is between an outer in-memory cache (e.g. moka) and the
/// raw state-store provider, so a process-lifetime hot path skips disk
/// entirely and only the first compile-then-deserialize crossing per template
/// per node ever pays the disk cost.
#[derive(Debug, Clone)]
pub struct DiskCachedWasmTemplateProvider<TStore> {
    inner: TStore,
    cache: WasmModuleCache,
}

impl<TStore> DiskCachedWasmTemplateProvider<TStore> {
    pub fn new(inner: TStore, cache: WasmModuleCache) -> Self {
        Self { inner, cache }
    }

    pub fn open(inner: TStore, path: impl Into<PathBuf>) -> io::Result<Self> {
        let wasm_cache = WasmModuleCache::open(path)?;
        Ok(Self::new(inner, wasm_cache))
    }
}

impl<TStore> TemplateProvider for DiskCachedWasmTemplateProvider<TStore>
where TStore: TemplateProvider<Template = PublishedTemplate> + Clone + 'static
{
    type Error = DiskCachedWasmTemplateProviderError;
    type Template = LoadedTemplate;

    fn get_template(&self, address: &TemplateAddress) -> Result<Option<Self::Template>, Self::Error> {
        // Builtins bypass the disk cache: their addresses are hardcoded
        // constants (independent of binary content), so a cache entry under
        // a builtin's address would silently serve an out-of-date compiled
        // module after a builtin recompile. User-template addresses are
        // content-addressed, so binary changes implicitly invalidate the
        // cache key.
        if is_builtin_template_address(address) {
            let Some(published) = self
                .inner
                .get_template(address)
                .map_err(|e| DiskCachedWasmTemplateProviderError::Inner(e.into()))?
            else {
                return Ok(None);
            };
            return Ok(Some(WasmModule::load_template_from_code(published.binary.as_slice())?));
        }

        if let Some(loaded) = self.cache.try_load(address) {
            return Ok(Some(loaded));
        }

        let Some(published) = self
            .inner
            .get_template(address)
            .map_err(|e| DiskCachedWasmTemplateProviderError::Inner(e.into()))?
        else {
            return Ok(None);
        };

        let loaded = WasmModule::load_template_from_code(published.binary.as_slice())?;
        self.cache.store(address, &loaded);
        Ok(Some(loaded))
    }

    fn has_template(&self, address: &TemplateAddress) -> Result<bool, Self::Error> {
        // Cheap path: cache hit implies the template exists. A miss falls
        // through to the inner provider, which is allowed to answer without
        // materialising the binary.
        if !is_builtin_template_address(address) && self.cache.path_for(address).exists() {
            return Ok(true);
        }
        self.inner
            .has_template(address)
            .map_err(|e| DiskCachedWasmTemplateProviderError::Inner(e.into()))
    }
}

impl<TStore> TemplateMetadataProvider for DiskCachedWasmTemplateProvider<TStore>
where TStore: TemplateProvider<Template = PublishedTemplate> + Clone + 'static
{
    fn get_template_metadata(&self, id: &TemplateAddress) -> Result<Option<TemplateProviderMetadata>, Self::Error> {
        // Metadata always reads from the underlying state store, never from
        // the disk cache (the cache only stores the compiled module, not the
        // PublishedTemplate's author / epoch / metadata_hash fields).
        let template = self
            .inner
            .get_template(id)
            .map_err(|e| DiskCachedWasmTemplateProviderError::Inner(e.into()))?;
        Ok(template.map(|t| TemplateProviderMetadata {
            author: t.author,
            binary_hash: t.to_binary_hash(),
            epoch: Epoch(t.at_epoch),
            metadata_hash: t.metadata_hash,
        }))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DiskCachedWasmTemplateProviderError {
    #[error("Inner template provider error: {0}")]
    Inner(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error(transparent)]
    TemplateLoader(#[from] TemplateLoaderError),
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tari_engine_types::published_template::PublishedTemplate;
    use tari_template_builtin::all_builtin_templates;
    use tari_template_lib::types::crypto::RistrettoPublicKeyBytes;
    use tempfile::TempDir;

    use super::*;

    #[derive(Clone)]
    struct StaticStore {
        templates: Arc<std::collections::HashMap<TemplateAddress, PublishedTemplate>>,
    }

    #[derive(Debug, thiserror::Error)]
    #[error("not found")]
    struct StaticStoreError;

    impl TemplateProvider for StaticStore {
        type Error = StaticStoreError;
        type Template = PublishedTemplate;

        fn get_template(&self, address: &TemplateAddress) -> Result<Option<Self::Template>, Self::Error> {
            Ok(self.templates.get(address).cloned())
        }
    }

    fn make_store() -> (StaticStore, TemplateAddress) {
        // We re-use the Account builtin's *binary* (it's a real, valid WASM
        // template available in dev-deps) but file it under a synthetic
        // non-builtin address — the disk-cache path bypasses real builtin
        // addresses by design (see is_builtin_template_address).
        let template = all_builtin_templates()
            .iter()
            .find(|t| t.name == "Account")
            .expect("Account builtin");
        let test_addr = TemplateAddress::from_array([
            0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42,
            0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42,
        ]);
        debug_assert!(
            !is_builtin_template_address(&test_addr),
            "test address must not collide with a builtin",
        );
        let mut map = std::collections::HashMap::new();
        let published = PublishedTemplate {
            template_name: template.name.try_into().expect("valid name"),
            author: RistrettoPublicKeyBytes::default(),
            binary: template.binary.to_vec().try_into().expect("template binary too large"),
            at_epoch: 0,
            metadata_hash: None,
        };
        map.insert(test_addr, published);
        (
            StaticStore {
                templates: Arc::new(map),
            },
            test_addr,
        )
    }

    #[test]
    fn round_trip_compile_then_deserialize() {
        let dir = TempDir::new().unwrap();
        let cache = WasmModuleCache::open(dir.path()).unwrap();
        let (store, addr) = make_store();
        let provider = DiskCachedWasmTemplateProvider::new(store.clone(), cache.clone());

        // First call: cache miss, compile-then-store.
        let first = provider.get_template(&addr).unwrap().expect("loaded");
        assert!(cache.path_for(&addr).exists(), "store should write a file");

        // Second call: cache hit, deserialize-only path.
        let second = provider.get_template(&addr).unwrap().expect("loaded");
        assert_eq!(first.template_name(), second.template_name());
        assert_eq!(first.code_size(), second.code_size());
        assert_eq!(
            first.template_def().functions().len(),
            second.template_def().functions().len(),
        );
    }

    #[test]
    fn corrupt_cache_falls_back_to_recompile() {
        let dir = TempDir::new().unwrap();
        let cache = WasmModuleCache::open(dir.path()).unwrap();
        let (store, addr) = make_store();

        // Plant garbage at the expected filename.
        let path = cache.path_for(&addr);
        fs::write(&path, b"this is not a wasmer artifact").unwrap();
        assert!(path.exists());

        // try_load should return None, having removed the corrupt file.
        assert!(cache.try_load(&addr).is_none());
        assert!(!path.exists(), "corrupt file should be removed");

        // Provider compiles fresh and writes a valid file.
        let provider = DiskCachedWasmTemplateProvider::new(store, cache.clone());
        provider.get_template(&addr).unwrap().expect("loaded");
        assert!(path.exists(), "fresh compile should re-populate the cache");

        // And the freshly-cached file deserializes cleanly.
        assert!(cache.try_load(&addr).is_some());
    }

    #[test]
    fn fingerprint_mismatch_treated_as_miss() {
        let dir = TempDir::new().unwrap();
        let cache = WasmModuleCache::open(dir.path()).unwrap();
        let (_store, addr) = make_store();

        // Plant a file under a different fingerprint suffix.
        let alt = dir.path().join(format!("{}_v0.bin", addr));
        fs::write(&alt, b"some bytes").unwrap();

        // Real path doesn't exist; try_load returns None and doesn't touch alt.
        assert!(cache.try_load(&addr).is_none());
        assert!(alt.exists(), "files for other fingerprints are left alone");
    }
}
