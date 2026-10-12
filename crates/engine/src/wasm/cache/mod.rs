//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! On-disk cache for compiled wasmer modules.
//!
//! Cranelift compilation of WASM templates is expensive: ~6 MB peak heap and
//! tens of milliseconds per template, paid on every node startup. The compiled
//! output for a given `(wasm_source, engine)` is deterministic and
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
//! - [`WasmModuleCache`] — low-level helper for callers that sit outside a `TemplateProvider` chain.
//! - [`DiskCachedWasmTemplateProvider`] — a `TemplateProvider` middleware that wraps a raw `PublishedTemplate` provider
//!   and outputs `LoadedTemplate`, doing compile-or-deserialize behind the scenes.

mod const_hash;

use std::{
    collections::HashMap,
    fs,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        Mutex,
        MutexGuard,
        PoisonError,
        atomic::{self, AtomicU64},
        mpsc,
    },
    thread,
    time::{Duration, SystemTime},
};

use log::*;
use memmap2::Mmap;
use tari_engine_types::{limits, limits::ModuleShape, published_template::PublishedTemplate};
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

const LOG_TARGET: &str = "tari::ootle::engine::wasm::cache";

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Engine-config fingerprint embedded in cache filenames, derived from everything that decides
/// what a compiled artifact contains and what is read back off it.
///
/// An artifact is only interchangeable with a fresh compile under the exact configuration that
/// produced it, and two of the values a hit serves verbatim — the shape counts that price
/// `instantiation_points`, and the metering global the artifact carries — are consensus figures.
/// A node serving an artifact built under different settings therefore charges a different fee for
/// the same transaction, or meters against a different cap, than one that compiled fresh. Deriving
/// the fingerprint is what ties the filename to those settings without anyone having to remember to
/// say so.
///
/// Every source input is a file narrow enough that each line in it decides what an artifact
/// contains, which is what [`engine`](super::engine) and
/// [`module_shape`](super::module_shape) exist as separate files for: a digest over a file is only
/// as precise as that file is narrow, and imprecision here is paid for by every node recompiling
/// every template. What is configuration rather than logic — the header layout and the limits — is
/// hashed as values instead, so restating it cannot drift from applying it.
///
/// Computed in a `const` context, so the digest is a compile-time constant and the source text it
/// reads never reaches the binary.
///
/// The wasmer version is the one input not derived here, because a crate cannot read its
/// dependencies' resolved versions and a published crate cannot reach the lockfile. It is hashed as
/// [`WASMER_VERSION`] instead, which a test holds to the workspace lockfile.
const ENGINE_FINGERPRINT_BITS: u64 = {
    let h = const_hash::init(b"tari.ootle.wasm_cache.engine_fingerprint.v2");
    // The compiler that generated every artifact's native code.
    let h = const_hash::part(h, WASMER_VERSION.as_bytes());
    // The compiler flags, feature set and tunables every artifact is built under.
    let h = const_hash::part(h, include_bytes!("../engine.rs"));
    // The derivation of the shape counts a hit serves verbatim out of the header.
    let h = const_hash::part(h, include_bytes!("../module_shape.rs"));
    // The per-operator cost tables the middlewares bake into the emitted instrumentation.
    let h = const_hash::part(h, include_bytes!("../metering.rs"));
    let h = const_hash::part(h, include_bytes!("../bulk_metering.rs"));
    // The operators and segments refused at compile, and the globals exported for restoring.
    let h = const_hash::part(h, include_bytes!("../instance_reset.rs"));
    // The memory and table bounds, which reach codegen through the style each one is adjusted to.
    let h = const_hash::part(h, include_bytes!("../limiting_tunable.rs"));
    // Which count a hit reads out of each header slot. Swapping two of these is a fee change.
    let h = const_hash::values(h, &header_layout());
    // Every limit, not just the ones the artifact bakes in: `validate_module_structure` enforces
    // `max_tables`, `max_globals` and the module and function limits on what the compile emits at
    // compile time only, and a hit goes straight to `finalize_loaded_module` without it.
    // Destructured rather than read field by field, so that a limit added later does not compile
    // until it is named here.
    let limits::WasmLimits {
        max_function_arguments,
        max_function_name_length,
        max_functions,
        max_module_functions,
        max_module_variables,
        max_function_metering_checks,
        max_function_loops,
        max_module_loops,
        max_function_nesting_depth,
        max_memory_pages,
        max_globals,
        max_tables,
        max_table_elements,
    } = limits::WASM_LIMITS;
    const_hash::finish(const_hash::values(h, &[
        limits::MAX_WASM_POINTS_PER_CALL as u128,
        max_function_arguments as u128,
        max_function_name_length as u128,
        max_functions as u128,
        max_module_functions as u128,
        max_module_variables as u128,
        max_function_metering_checks as u128,
        max_function_loops as u128,
        max_module_loops as u128,
        max_function_nesting_depth as u128,
        max_memory_pages as u128,
        max_globals as u128,
        max_tables as u128,
        max_table_elements as u128,
        // The compile work `validate_module_structure` refuses a module for is these counts at these
        // prices, against the size cap.
        limits::ENGINE_LIMITS.max_template_binary_size_bytes as u128,
        limits::TEMPLATE_COMPILE_BYTES_PER_FUNCTION as u128,
        limits::TEMPLATE_COMPILE_BYTES_PER_VARIABLE as u128,
        limits::TEMPLATE_COMPILE_BYTES_PER_METERING_CHECK as u128,
        limits::TEMPLATE_COMPILE_FUNCTION_CHECKS_SQUARED_PER_BYTE as u128,
    ]))
};

/// The resolved `wasmer` version. It decides the codegen as well as the artifact format: wasmer pins
/// cranelift at an exact version, and its `MetadataHeader` version moves only when the format does,
/// so a release that changes only the generated code still loads every artifact the previous one
/// wrote. Two nodes that differ only in cache history then run different code for the same template.
const WASMER_VERSION: &str = "7.5.0";

/// The fingerprint as the lowercase hex it appears in a filename as.
///
/// Short enough for a filename, wide enough that no node will see two configurations collide. The
/// hash is not cryptographic, and the property wanted of it is separation rather than resistance:
/// every input is this build's own source and constants, so there is no party to search for a
/// collision.
static ENGINE_FINGERPRINT_HEX: [u8; 16] = const_hash::to_hex(ENGINE_FINGERPRINT_BITS);

pub static ENGINE_FINGERPRINT: &str = match str::from_utf8(&ENGINE_FINGERPRINT_HEX) {
    Ok(hex) => hex,
    Err(_) => panic!("hex digits are ASCII"),
};

/// An 8-byte little-endian field at the head of a cache file.
///
/// The order of [`HEADER_FIELDS`] is the file format: it decides which count a hit reads each slot
/// into. Both the write and the read are driven from that array rather than repeating it, so the
/// order the fingerprint hashes is the order the file is actually written and parsed in.
#[derive(Clone, Copy)]
enum HeaderField {
    /// The original WASM source byte count, which `wasmer::Module::serialize` does not preserve
    /// and downstream caches weigh by.
    CodeSize,
    DataSegmentBytes,
    DataSegmentCount,
    ElementSegmentEntries,
    DeclaredTableSlots,
}

const HEADER_FIELDS: [HeaderField; 5] = [
    HeaderField::CodeSize,
    HeaderField::DataSegmentBytes,
    HeaderField::DataSegmentCount,
    HeaderField::ElementSegmentEntries,
    HeaderField::DeclaredTableSlots,
];

impl HeaderField {
    const fn read_from(self, code_size: usize, shape: &ModuleShape) -> u64 {
        match self {
            HeaderField::CodeSize => code_size as u64,
            HeaderField::DataSegmentBytes => shape.data_segment_bytes,
            HeaderField::DataSegmentCount => shape.data_segment_count,
            HeaderField::ElementSegmentEntries => shape.element_segment_entries,
            HeaderField::DeclaredTableSlots => shape.declared_table_slots,
        }
    }

    fn write_into(self, value: u64, code_size: &mut usize, shape: &mut ModuleShape) {
        match self {
            HeaderField::CodeSize => *code_size = value as usize,
            HeaderField::DataSegmentBytes => shape.data_segment_bytes = value,
            HeaderField::DataSegmentCount => shape.data_segment_count = value,
            HeaderField::ElementSegmentEntries => shape.element_segment_entries = value,
            HeaderField::DeclaredTableSlots => shape.declared_table_slots = value,
        }
    }
}

/// A shape whose every field is a different number, and a code size unlike any of them.
///
/// Written out field by field rather than built from a default, so a field added to `ModuleShape`
/// does not compile until it is given a witness value here.
const LAYOUT_WITNESS_SHAPE: ModuleShape = ModuleShape {
    data_segment_bytes: 0x11,
    data_segment_count: 0x22,
    element_segment_entries: 0x33,
    declared_table_slots: 0x44,
};
const LAYOUT_WITNESS_CODE_SIZE: usize = 0x55;

/// What each header slot means, as the slot itself answers it.
///
/// Reading the witness back through [`HeaderField::read_from`] yields one witness value per slot,
/// in slot order. Reordering the slots permutes it and repurposing one replaces a value, so the
/// sequence the fingerprint hashes is produced by the accessor a hit actually uses rather than
/// restated alongside it, and the two cannot drift.
const fn header_layout() -> [u128; HEADER_FIELD_COUNT] {
    let mut layout = [0u128; HEADER_FIELD_COUNT];
    let mut i = 0;
    while i < HEADER_FIELD_COUNT {
        layout[i] = HEADER_FIELDS[i].read_from(LAYOUT_WITNESS_CODE_SIZE, &LAYOUT_WITNESS_SHAPE) as u128;
        i += 1;
    }
    layout
}

/// The 8-byte LE fields at the head of each cache file. `wasmer::Module::serialize` preserves none
/// of them, and all are needed after a cache hit — the first for accounting (e.g. moka weighing),
/// the rest to price instantiation.
const HEADER_FIELD_COUNT: usize = HEADER_FIELDS.len();
const HEADER_FIELD_BYTES: usize = HEADER_FIELD_COUNT * 8;

/// Zero padding between the fields and the tag. This is the knob that satisfies the alignment
/// assert below when the field count changes; the tag stays a `u64`.
const HEADER_PAD_BYTES: usize = 0;

/// Offset of the 8-byte LE integrity tag.
///
/// It covers the header fields, the artifact body, and the file's identity — the template address
/// and the engine fingerprint. The fields sit outside the wasmer artifact, so
/// `deserialize_unchecked` has no view of damage confined to them, while the four shape counts
/// price `instantiation_points` into a committed fee receipt. The body needs its own cover because
/// wasmer validates only the 32 bytes of prefix it needs to find the metadata: past that,
/// `rkyv::access_unchecked` reads whatever is there, and a body failing the prefix check is handed
/// to an ELF loader rather than refused. Binding the address and the fingerprint is what makes a
/// file answer for the name it was found under.
const TAG_OFFSET: usize = HEADER_FIELD_BYTES + HEADER_PAD_BYTES;

/// The header fields, the pad and the tag.
///
/// The total is a multiple of 16: the artifact starts at `HEADER_BYTES` into a page-aligned mmap
/// and its rkyv metadata a further 32 bytes in, where `rkyv::access_unchecked` reads an archived
/// root that wasmer aligns to `MetadataHeader::ALIGN` (16); `MetadataHeader::parse` enforces the
/// weaker 8-byte bound on the artifact itself.
const HEADER_BYTES: usize = TAG_OFFSET + 8;

/// The integrity tag over a cache file: its identity, its header fields and its artifact body.
///
/// This is a checksum, not an authenticator. It turns a damaged or swapped file into a miss and a
/// recompile, which is what keeps bit rot and a stale restore from reaching
/// `deserialize_unchecked`. It is no obstacle to a process that can write the cache directory,
/// since such a process can write a matching tag; what addresses that is the directory's mode,
/// which [`WasmModuleCache::open`] takes away from group and other.
fn integrity_tag(addr: &TemplateAddress, header_fields: &[u8], body: &[u8]) -> u64 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(ENGINE_FINGERPRINT.as_bytes());
    hasher.update(addr.as_ref());
    hasher.update(header_fields);
    hasher.update(body);
    u64::from(hasher.finalize())
}

const _: () = assert!(
    HEADER_BYTES.is_multiple_of(16),
    "the artifact's rkyv metadata must start 16-byte aligned: widen HEADER_PAD_BYTES",
);

/// Infix marking a `store` tempfile, which no reader ever names.
const TMP_INFIX: &str = ".bin.tmp.";

/// How long a tempfile must have gone untouched before [`WasmModuleCache::open`] treats it as the
/// residue of a run that died between its write and its rename. A live `store` publishes in
/// milliseconds, so the margin only has to clear a stalled flush.
const STALE_TEMPFILE_AGE: Duration = Duration::from_secs(300);

/// What the cache knows about one file it has seen.
#[derive(Debug)]
struct IndexEntry {
    size_bytes: u64,
    /// Tick of the entry's last hit or store. Only the ordering carries meaning.
    last_used: u64,
    /// Identity of the file this entry describes, on platforms that expose one. An unlink checks it
    /// so that a file a concurrent `store` has since renamed into place is left alone.
    identity: Option<u64>,
}

/// Bookkeeping over the cache directory: what is in it, how much it weighs, and in what order it
/// was last wanted.
///
/// The directory remains the authority on contents. This is a running tally kept so that `store`
/// can answer "am I over the cap, and what is coldest?" without a `readdir` and a `stat` per file
/// on the execution path. A tally that drifts from the directory costs a recompile, never
/// correctness: an entry the index has lost is simply an unaccounted file, and one it invents is
/// dropped the next time its unlink finds nothing.
#[derive(Debug)]
struct CacheIndex {
    entries: HashMap<TemplateAddress, IndexEntry>,
    total_bytes: u64,
    cap_bytes: u64,
    clock: u64,
}

impl CacheIndex {
    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    fn record(&mut self, addr: TemplateAddress, size_bytes: u64, identity: Option<u64>) {
        let last_used = self.tick();
        if let Some(previous) = self.entries.insert(addr, IndexEntry {
            size_bytes,
            last_used,
            identity,
        }) {
            self.total_bytes = self.total_bytes.saturating_sub(previous.size_bytes);
        }
        self.total_bytes = self.total_bytes.saturating_add(size_bytes);
    }

    fn forget(&mut self, addr: &TemplateAddress) {
        if let Some(entry) = self.entries.remove(addr) {
            self.total_bytes = self.total_bytes.saturating_sub(entry.size_bytes);
        }
    }

    /// Removes and returns the least recently used entry.
    ///
    /// The last entry is never taken: a single artifact larger than the whole cap would otherwise be
    /// evicted by the very `store` that wrote it, and every call would recompile it.
    fn take_coldest(&mut self) -> Option<(TemplateAddress, IndexEntry)> {
        if self.entries.len() <= 1 {
            return None;
        }
        let addr = *self
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(addr, _)| addr)?;
        let entry = self.entries.remove(&addr)?;
        self.total_bytes = self.total_bytes.saturating_sub(entry.size_bytes);
        Some((addr, entry))
    }
}

/// The template an artifact filename names, if it is this build's.
fn parse_artifact_name(name: &str, suffix: &str) -> Option<TemplateAddress> {
    TemplateAddress::from_hex(name.strip_suffix(suffix)?).ok()
}

/// The template an artifact filename names, if it belongs to a build with a different
/// [`ENGINE_FINGERPRINT`].
///
/// The shape is `{template address}_{fingerprint}.bin`. A name this does not parse belongs to
/// something other than this cache and is left where it is.
fn parse_orphan_name(name: &str) -> Option<TemplateAddress> {
    let stem = name.strip_suffix(".bin")?;
    let (addr, fingerprint) = stem.rsplit_once('_')?;
    if fingerprint == ENGINE_FINGERPRINT {
        return None;
    }
    TemplateAddress::from_hex(addr).ok()
}

/// Takes write permission on the cache directory away from group and other.
///
/// Whoever can write a file here chooses the native code this process executes: an artifact goes to
/// `Module::deserialize_unchecked`, which reads a body past its 32-byte prefix with
/// `rkyv::access_unchecked` and hands one that fails the prefix check to an ELF loader. The
/// integrity tag does not help — a writer can write a matching one — so the mode is what makes the
/// directory's contents this process's own.
///
/// Hardening, so it reports rather than fails. A directory owned by another uid — an earlier run as
/// root against the same mounted data dir, say — cannot be chmod'd, and the choice there is between
/// running against a directory this process does not exclusively own and not starting at all. The
/// first is the lesser of the two, and it is what every caller gets. A deployment that shares this
/// directory between two accounts through a group loses the second account's writes, which is the
/// point rather than a side effect.
#[cfg(unix)]
fn restrict_dir_to_owner(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;

    /// What the node runs with when the mode cannot be read or cannot be changed.
    fn warn_unrestricted(dir: &Path, e: io::Error) {
        warn!(
            target: LOG_TARGET,
            "Could not restrict the Wasm module cache at {} to its owner: {}. Continuing: anything \
             able to write there chooses the native code this node runs.",
            dir.display(),
            e,
        );
    }

    let mode = match fs::metadata(dir) {
        Ok(meta) => meta.permissions().mode(),
        Err(e) => return warn_unrestricted(dir, e),
    };
    if mode & 0o022 == 0 {
        return;
    }
    match fs::set_permissions(dir, fs::Permissions::from_mode(mode & !0o022)) {
        Ok(_) => warn!(
            target: LOG_TARGET,
            "Wasm module cache at {} was writable beyond its owner (mode {:#o}); restricted it to {:#o}.",
            dir.display(),
            mode & 0o7777,
            mode & 0o7777 & !0o022,
        ),
        Err(e) => warn_unrestricted(dir, e),
    }
}

#[cfg(not(unix))]
fn restrict_dir_to_owner(_dir: &Path) {}

/// The identity a later unlink compares against, where the platform has one.
#[cfg(unix)]
fn identity_of(meta: &fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(meta.ino())
}

#[cfg(not(unix))]
fn identity_of(_meta: &fs::Metadata) -> Option<u64> {
    None
}

/// Unlink `path`, unless it now names a different file than `identity` describes.
///
/// Both eviction and the corrupt-file paths delete by path, while `store` publishes by renaming a
/// new inode over that same path, so a delete decided against the file one call read can land on the
/// file another call has just published. The check narrows that window to the gap between the stat
/// and the unlink, on platforms that expose an identity; elsewhere the unlink is unconditional. What
/// is left costs a recompile.
fn remove_tracked_file(path: &Path, identity: Option<u64>) {
    if let Some(identity) = identity {
        match fs::metadata(path) {
            Ok(meta) if identity_of(&meta) != Some(identity) => {
                debug!(
                    target: LOG_TARGET,
                    "Leaving {} alone: it was replaced since it was read", path.display(),
                );
                return;
            },
            Ok(_) => {},
            Err(_) => return,
        }
    }
    let _ignore = fs::remove_file(path);
}

/// Low-level on-disk cache for compiled wasmer modules.
///
/// Files live at `{dir}/{template_address}_{ENGINE_FINGERPRINT}.bin`.
/// The body is `[u64 LE: code_size][u64 LE x4: ModuleShape][u64 LE: integrity tag]
/// || wasmer::Module::serialize(...)`, where the tag is [`integrity_tag`].
///
/// Writes are atomic (tempfile + rename). Read failures (missing file,
/// deserialize errors, format changes) are non-fatal: the corrupt file is
/// removed and the caller is expected to recompile from source.
///
/// The cache is bounded by total bytes rather than by entry count: a compiled artifact runs about
/// ten times the size of the WASM it came from, and the range across real templates is wide enough
/// (hundreds of KiB to tens of MiB) that a count says little about disk. Over the cap, the least
/// recently used artifacts are unlinked.
///
/// Eviction is node-local policy with no consensus reach, because an evicted artifact costs only a
/// recompile of bytes that remain on-chain.
///
/// [`WasmModuleCache::store`] hands its artifact to a writer thread instead of writing it. Writing
/// one means serializing a multi-megabyte artifact and flushing it to the device, and `store`'s
/// caller is an execution: on a validator that is consensus, on the indexer a dry run. What the
/// caller needs from a store is that the artifact is on disk before it is wanted again, which is
/// the next transaction to call the template at the earliest, not before the call that compiled it
/// returns.
#[derive(Debug, Clone)]
pub struct WasmModuleCache {
    dir: PathBuf,
    index: Arc<Mutex<CacheIndex>>,
    writes: mpsc::SyncSender<WriteRequest>,
}

/// Artifacts the writer thread will accept before [`WasmModuleCache::store`] starts dropping them.
///
/// Queueing one is a cheap clone — a `wasmer::Module` is an `Arc` — but the queue outlives the
/// execution that compiled it, so a full queue is the last owner of this many artifacts, which this
/// module's own docs put at hundreds of KiB to tens of MiB each. The queue's job is to decouple a
/// caller from one slow write rather than to hold a cache's worth of compiled code, so it is sized
/// for the first: a backlog only forms from compiles running at once, since every store follows a
/// compile that costs more than the write does, and this sits above the number of those a node has
/// cores for. A dropped artifact costs a recompile the next time its template is wanted cold.
const WRITE_QUEUE_CAPACITY: usize = 16;

/// A request to the writer thread.
enum WriteRequest {
    Artifact {
        addr: TemplateAddress,
        loaded: LoadedTemplate,
    },
    /// Answered once every artifact queued before it has been written.
    #[cfg(test)]
    Barrier(mpsc::SyncSender<()>),
}

impl WasmModuleCache {
    /// Open or create a cache rooted at `dir`, holding at most `cap_bytes` of artifacts. Creates the
    /// directory tree if missing.
    ///
    /// A directory takes one instance per process, cloned to each of its consumers rather than opened again by
    /// each, so that what the handle tracks covers the whole directory.
    ///
    /// Opening walks the directory to tally what is already there, and takes the chance to clear
    /// tempfiles no live `store` is writing to. Anything over the cap — a cache from a run
    /// configured with a larger one — is evicted before the first lookup.
    pub fn open(dir: impl Into<PathBuf>, cap_bytes: u64) -> io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        restrict_dir_to_owner(&dir);
        let (writes, requests) = mpsc::sync_channel(WRITE_QUEUE_CAPACITY);
        let cache = Self {
            index: Arc::new(Mutex::new(Self::build_index(&dir, cap_bytes)?)),
            dir,
            writes,
        };
        cache.evict_to_fit();
        cache.spawn_writer(requests);
        // A default that decides how much disk this node uses and what it deletes is logged
        // unconditionally: an operator who upgraded without touching their config should be able to
        // see what changed underneath them in their own logs.
        info!(
            target: LOG_TARGET,
            "⚙️ Wasm module cache at {} holds {} artifact(s), {} bytes of a {} byte cap",
            cache.dir.display(),
            cache.len(),
            cache.total_bytes(),
            cap_bytes,
        );
        Ok(cache)
    }

    /// Tallies the cache directory, and clears what no build can use: tempfiles no live `store` is
    /// writing, and artifacts under a fingerprint other than this build's.
    ///
    /// Reaping other fingerprints is what makes the cap a bound on the directory rather than on one
    /// generation of it. A fingerprint bump follows any wasmer upgrade or metering change, and the
    /// build that wrote the previous generation is the one being replaced, so nothing else will ever
    /// collect those files. The cost of getting it wrong — a directory shared with an older binary,
    /// which then finds its artifacts gone — is a recompile per template on that binary's next run.
    ///
    /// Recency is seeded from modification time, the only ordering the filesystem retains across a
    /// restart. Hits are not written back to it: a `utimes` per cache hit buys a better order after
    /// the next restart at the cost of a write on every read, and a mis-ordered eviction costs one
    /// recompile.
    fn build_index(dir: &Path, cap_bytes: u64) -> io::Result<CacheIndex> {
        let suffix = format!("_{}.bin", ENGINE_FINGERPRINT);
        let stale_before = SystemTime::now().checked_sub(STALE_TEMPFILE_AGE);
        let mut found = Vec::new();

        for entry in fs::read_dir(dir)? {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }

            // A tempfile is a partial write, valid for no build, so the age gate is the only
            // question: a live `store` may still be writing it.
            if name.contains(TMP_INFIX) {
                let is_abandoned =
                    stale_before.is_some_and(|stale_before| meta.modified().is_ok_and(|m| m < stale_before));
                if is_abandoned {
                    debug!(target: LOG_TARGET, "Reaping abandoned cache tempfile {}", name);
                    let _ignore = fs::remove_file(entry.path());
                }
                continue;
            }

            let Some(addr) = parse_artifact_name(name, &suffix) else {
                if let Some(addr) = parse_orphan_name(name) {
                    debug!(
                        target: LOG_TARGET,
                        "Reaping cached module for template {} under a superseded fingerprint", addr,
                    );
                    let _ignore = fs::remove_file(entry.path());
                }
                continue;
            };
            found.push((
                addr,
                meta.len(),
                meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                identity_of(&meta),
            ));
        }

        found.sort_by_key(|(_, _, modified, _)| *modified);

        let mut index = CacheIndex {
            entries: HashMap::with_capacity(found.len()),
            total_bytes: 0,
            cap_bytes,
            clock: 0,
        };
        for (addr, size_bytes, _, identity) in found {
            index.record(addr, size_bytes, identity);
        }
        Ok(index)
    }

    /// A poisoned index is recovered rather than propagated: it is a tally over a directory that
    /// remains the authority, so the worst a panic mid-update leaves behind is a wrong byte total.
    fn index(&self) -> MutexGuard<'_, CacheIndex> {
        self.index.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Unlinks least-recently-used artifacts until the cache is within its cap.
    fn evict_to_fit(&self) {
        // The unlinks run outside the lock: only the tally has to be consistent, and a victim is
        // already out of it by the time its file goes.
        let mut victims = Vec::new();
        {
            let mut index = self.index();
            while index.total_bytes > index.cap_bytes {
                let Some(victim) = index.take_coldest() else {
                    break;
                };
                victims.push(victim);
            }
        }
        for (addr, entry) in victims {
            debug!(
                target: LOG_TARGET,
                "Evicting cached module for template {} ({} bytes)", addr, entry.size_bytes,
            );
            remove_tracked_file(&self.path_for(&addr), entry.identity);
        }
    }

    /// Drops `addr` from the index and unlinks its file, if that file is still the one `identity`
    /// describes.
    fn discard(&self, addr: &TemplateAddress, path: &Path, identity: Option<u64>) {
        self.index().forget(addr);
        remove_tracked_file(path, identity);
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
    /// hand it the mmap region without copying. A hit costs one `mmap` syscall
    /// and one pass over the mapping to verify the tag, which faults in every
    /// page ahead of the deserializer; no full-artifact allocation.
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
        let identity = file.metadata().ok().as_ref().and_then(identity_of);

        // SAFETY: see the docs on `Mmap::map`. A mapping SIGBUSes if the file shrinks under it, so
        // what keeps this sound is that `store` never writes through the published path: it fills a
        // tempfile only that call can name and renames, pointing the directory entry at a new inode
        // and leaving this mapping's inode whole. Writers under a different engine config target a
        // different filename through the fingerprint suffix.
        let mmap = match unsafe { Mmap::map(&file) } {
            Ok(m) => m,
            Err(e) => {
                warn!(
                    target: LOG_TARGET,
                    "Failed to mmap cache file {}: {}", path.display(), e,
                );
                // An empty file cannot be mapped at all, so it never reaches the length check below.
                // Other map failures are resource limits, under which the file may still be good.
                if file.metadata().is_ok_and(|m| m.len() == 0) {
                    self.discard(addr, &path, identity);
                }
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
            self.discard(addr, &path, identity);
            return None;
        }

        let stored_tag = u64::from_le_bytes(
            mmap[TAG_OFFSET..HEADER_BYTES]
                .try_into()
                .expect("HEADER_BYTES - TAG_OFFSET is 8"),
        );
        let computed_tag = integrity_tag(addr, &mmap[..TAG_OFFSET], &mmap[HEADER_BYTES..]);
        if stored_tag != computed_tag {
            warn!(
                target: LOG_TARGET,
                "Cache file {} failed its integrity tag ({:#010x}, expected {:#010x}); removing.",
                path.display(),
                stored_tag,
                computed_tag,
            );
            drop(mmap);
            self.discard(addr, &path, identity);
            return None;
        }

        let mut code_size = 0usize;
        let mut shape = ModuleShape::default();
        for (slot, field) in mmap[..HEADER_FIELD_BYTES].chunks_exact(8).zip(HEADER_FIELDS) {
            let value = u64::from_le_bytes(slot.try_into().expect("chunks_exact(8) yields 8 bytes"));
            field.write_into(value, &mut code_size, &mut shape);
        }

        // Wrap the mmap as a Bytes that owns it, then slice past the
        // header. `Bytes::slice` is zero-copy (pointer + length
        // adjustment); the wrapped Mmap is dropped only when the resulting
        // Bytes (and any clones the deserializer may keep) goes out of
        // scope.
        let size_bytes = mmap.len() as u64;
        let body = bytes::Bytes::from_owner(mmap).slice(HEADER_BYTES..);

        // SAFETY: these bytes carry an [`integrity_tag`] computed over this node's
        // [`ENGINE_FINGERPRINT`], the address they are filed under, the header fields and the body,
        // and the tag was checked above. Only a writer holding this directory can produce a
        // matching one, which is what [`restrict_dir_to_owner`] is for: `deserialize_unchecked`
        // reads a body past its 32-byte prefix with `rkyv::access_unchecked` and hands one that
        // fails the prefix check to an ELF loader, so bytes that reach it are already trusted.
        match unsafe { WasmModule::load_template_from_serialized(body, code_size, shape) } {
            Ok(loaded) => {
                debug!(target: LOG_TARGET, "Cache hit for template {}", addr);
                // A hit also adopts a file this instance never wrote, so a cache inherited from an
                // earlier run counts towards the cap from the first time it is wanted.
                self.index().record(*addr, size_bytes, identity);
                Some(loaded)
            },
            Err(err) => {
                warn!(
                    target: LOG_TARGET,
                    "Failed to deserialize cached module {}: {}; removing.",
                    path.display(),
                    err,
                );
                self.discard(addr, &path, identity);
                None
            },
        }
    }

    /// One thread per cache, so that artifacts are written in the order they were offered and two
    /// stores of one address never race for its filename.
    ///
    /// It ends when the cache and every clone of it are dropped, which closes the queue. An
    /// artifact still queued at that point is lost, and costs the recompile any artifact that was
    /// never written costs.
    fn spawn_writer(&self, requests: mpsc::Receiver<WriteRequest>) {
        let cache = self.clone_without_writer();
        let spawned = thread::Builder::new()
            .name("wasm-cache-writer".to_string())
            .spawn(move || {
                for request in requests {
                    match request {
                        WriteRequest::Artifact { addr, loaded } => cache.write_now(&addr, &loaded),
                        #[cfg(test)]
                        WriteRequest::Barrier(reply) => {
                            let _ignore = reply.send(());
                        },
                    }
                }
                debug!(target: LOG_TARGET, "Wasm module cache writer exiting");
            });
        if let Err(e) = spawned {
            warn!(
                target: LOG_TARGET,
                "Failed to spawn the wasm module cache writer: {}; artifacts will not be cached", e,
            );
        }
    }

    /// A handle onto the same directory and index whose own `writes` sender is disconnected.
    ///
    /// The writer thread holds this rather than a full clone: a clone would keep the queue's sender
    /// alive from inside the thread that drains it, so the queue would never close and the thread
    /// would never end.
    fn clone_without_writer(&self) -> Self {
        let (writes, _dropped) = mpsc::sync_channel(0);
        Self {
            dir: self.dir.clone(),
            index: self.index.clone(),
            writes,
        }
    }

    /// Offer a compiled module to the cache under `addr`.
    ///
    /// Returns as soon as the artifact is queued for the writer thread. Best-effort throughout: a
    /// full queue drops the artifact and any failure to write it is logged, because the caller's
    /// compiled module is valid either way and the only cost of no cache entry is a later
    /// recompile.
    pub fn store(&self, addr: &TemplateAddress, loaded: &LoadedTemplate) {
        let request = WriteRequest::Artifact {
            addr: *addr,
            loaded: loaded.clone(),
        };
        match self.writes.try_send(request) {
            Ok(_) => {},
            Err(mpsc::TrySendError::Full(_)) => {
                // Warned rather than logged quietly: a device slow enough to keep the writer behind
                // turns this cache into a directory nothing lands in, and every call to a cold
                // template recompiles it. Before the write moved off the caller that showed up as
                // slow execution, which an operator could see.
                warn!(
                    target: LOG_TARGET,
                    "The wasm module cache writer is behind. Dropping the compiled module for template {}, which \
                     will be compiled again when that template is next wanted cold.",
                    addr,
                );
            },
            Err(mpsc::TrySendError::Disconnected(_)) => {
                warn!(
                    target: LOG_TARGET,
                    "The wasm module cache writer has stopped. Nothing further will be cached for the life of this \
                     process, starting with the compiled module for template {}.",
                    addr,
                );
            },
        }
    }

    /// Serialize `loaded` and publish it at `addr`'s cache filename.
    fn write_now(&self, addr: &TemplateAddress, loaded: &LoadedTemplate) {
        let LoadedTemplate::Wasm(wasm) = loaded;
        let serialized = match wasm.wasm_module().serialize() {
            Ok(s) => s,
            Err(e) => {
                warn!(target: LOG_TARGET, "Failed to serialize module for {}: {}", addr, e);
                return;
            },
        };

        let path = self.path_for(addr);
        // Every call needs its own tempfile: another process pointing at this directory can be
        // publishing the same address at the same moment. The pid separates this process's writes
        // from theirs and the counter separates this call from the next.
        let tmp = self.dir.join(format!(
            "{}_{}.bin.tmp.{}.{}",
            addr,
            ENGINE_FINGERPRINT,
            std::process::id(),
            TMP_COUNTER.fetch_add(1, atomic::Ordering::Relaxed),
        ));

        let shape = wasm.shape();
        let mut header = [0u8; HEADER_BYTES];
        for (slot, field) in header.chunks_exact_mut(8).zip(HEADER_FIELDS) {
            slot.copy_from_slice(&field.read_from(wasm.code_size(), &shape).to_le_bytes());
        }
        let tag = integrity_tag(addr, &header[..TAG_OFFSET], &serialized);
        header[TAG_OFFSET..].copy_from_slice(&tag.to_le_bytes());

        let mut bytes = Vec::with_capacity(HEADER_BYTES + serialized.len());
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(&serialized);

        let identity = match write_durable(&tmp, &bytes) {
            Ok(identity) => identity,
            Err(e) => {
                warn!(target: LOG_TARGET, "Failed to write cache tempfile {}: {}", tmp.display(), e);
                let _ignore = fs::remove_file(&tmp);
                return;
            },
        };

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

        self.index().record(*addr, bytes.len() as u64, identity);
        self.evict_to_fit();
    }

    /// Block until every artifact offered before this call has been written.
    #[cfg(test)]
    fn flush(&self) {
        let (reply, done) = mpsc::sync_channel(0);
        if self.writes.send(WriteRequest::Barrier(reply)).is_ok() {
            let _ignore = done.recv();
        }
    }

    /// Bytes of artifact this cache is tracking.
    pub fn total_bytes(&self) -> u64 {
        self.index().total_bytes
    }

    /// Artifacts this cache is tracking.
    pub fn len(&self) -> usize {
        self.index().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Write `bytes` to `path`, flushed to the device before returning, and report the identity of the
/// file written.
///
/// [`WasmModuleCache::store`] publishes a file by rename, which can name contents still held in the
/// page cache. A torn artifact is only ever a tag mismatch and a recompile, so this buys latency
/// rather than safety: without it, a crash leaves a file that is found, mmap'd and hashed in full
/// before it is discarded. Durability stops at the contents — a rename lost to a crash costs one
/// recompile.
fn write_durable(path: &Path, bytes: &[u8]) -> io::Result<Option<u64>> {
    use std::io::Write;

    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    // Taken before the rename: the identity travels with the inode, and reading it here cannot race
    // another call publishing over the destination.
    Ok(file.metadata().ok().as_ref().and_then(identity_of))
}

/// `TemplateProvider` middleware that adds an on-disk compiled-module cache
/// behind any provider returning raw [`PublishedTemplate`] bytes.
///
/// On `get_template(addr)`:
/// 1. If the cache file `{addr}_{ENGINE_FINGERPRINT}.bin` exists, deserialize and return — no compile, no
///    inner-provider call.
/// 2. Otherwise delegate to `inner` for the raw `PublishedTemplate`, compile via
///    [`WasmModule::load_template_from_code`], offer the compiled module to the cache, return. The artifact is written
///    by the cache's writer thread, so the caller pays the compile but not the flush.
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

    /// Opens a cache at `path`, bounded to `cap_bytes`, for this provider's exclusive use. A process whose cache
    /// directory has another consumer opens it once with [`WasmModuleCache::open`] and passes a clone to
    /// [`Self::new`].
    pub fn open(inner: TStore, path: impl Into<PathBuf>, cap_bytes: u64) -> io::Result<Self> {
        let wasm_cache = WasmModuleCache::open(path, cap_bytes)?;
        Ok(Self::new(inner, wasm_cache))
    }
}

/// Both lookups below answer from a cache hit without asking `inner`. What makes that sound is a
/// property of the directory rather than of this type: every artifact in it is one whose template
/// substate had been resolved before it was written, and templates are immutable and are never
/// destroyed, so an artifact that was right when written stays right.
///
/// This provider is the directory's only writer, and it stores only what `inner.get_template` has
/// just served. It resolves builtin addresses without consulting the cache at all — their addresses are constants
/// rather than hashes of their binaries, so an artifact filed under one would outlive the binary it
/// came from.
///
/// Any other writer owes this path the same guarantee. `call_function` resolves a template through the
/// provider and nothing else, so a node serving an artifact for a substate that does not exist
/// executes a call every other node aborts.
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

    #[test]
    fn wasmer_version_matches_the_lockfile() {
        let lockfile = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock"))
            .expect("workspace Cargo.lock");
        let resolved = lockfile
            .split("[[package]]")
            .find(|entry| entry.lines().any(|line| line == r#"name = "wasmer""#))
            .and_then(|entry| entry.lines().find_map(|line| line.strip_prefix(r#"version = ""#)))
            .and_then(|version| version.strip_suffix('"'))
            .expect("wasmer in Cargo.lock");
        assert_eq!(
            WASMER_VERSION, resolved,
            "WASMER_VERSION must name the wasmer the workspace resolves, so that artifacts compiled by another \
             release are not served"
        );
    }

    /// Comfortably larger than the one artifact the shared fixtures compile, so that a test that is
    /// not about eviction never trips it.
    const TEST_CAP_BYTES: u64 = 64 * 1024 * 1024;

    fn account_binary() -> &'static [u8] {
        all_builtin_templates()
            .iter()
            .find(|t| t.name == "Account")
            .expect("Account builtin")
            .binary
    }

    fn addr_of_byte(b: u8) -> TemplateAddress {
        let addr = TemplateAddress::from_array([b; 32]);
        debug_assert!(
            !is_builtin_template_address(&addr),
            "test address must not collide with a builtin",
        );
        addr
    }

    /// One compiled artifact, and the byte count a `store` of it writes.
    fn compiled_artifact() -> (LoadedTemplate, u64) {
        let dir = TempDir::new().unwrap();
        let cache = WasmModuleCache::open(dir.path(), TEST_CAP_BYTES).unwrap();
        let addr = addr_of_byte(0x01);
        let loaded = WasmModule::load_template_from_code(account_binary()).unwrap();
        cache.store(&addr, &loaded);
        cache.flush();
        let size = fs::metadata(cache.path_for(&addr)).unwrap().len();
        (loaded, size)
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
        let cache = WasmModuleCache::open(dir.path(), TEST_CAP_BYTES).unwrap();
        let (store, addr) = make_store();
        let provider = DiskCachedWasmTemplateProvider::new(store.clone(), cache.clone());

        // First call: cache miss, compile-then-store.
        let first = provider.get_template(&addr).unwrap().expect("loaded");
        cache.flush();
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
    fn every_header_slot_reads_back_the_field_it_wrote() {
        // [`ENGINE_FINGERPRINT`] witnesses each slot through `read_from` alone. This is what holds
        // `write_into` to the same field, so that a hit puts a count back where the writer took it
        // from: every other field starts at zero, so a slot that crossed over reads one back.
        for field in HEADER_FIELDS {
            let expected = field.read_from(LAYOUT_WITNESS_CODE_SIZE, &LAYOUT_WITNESS_SHAPE);
            let mut code_size = 0;
            let mut shape = ModuleShape::default();
            field.write_into(expected, &mut code_size, &mut shape);
            assert_eq!(field.read_from(code_size, &shape), expected);
        }
    }

    #[test]
    fn corrupt_cache_falls_back_to_recompile() {
        let dir = TempDir::new().unwrap();
        let cache = WasmModuleCache::open(dir.path(), TEST_CAP_BYTES).unwrap();
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
        cache.flush();
        assert!(path.exists(), "fresh compile should re-populate the cache");

        // And the freshly-cached file deserializes cleanly.
        assert!(cache.try_load(&addr).is_some());
    }

    #[test]
    fn flipped_header_byte_falls_back_to_recompile() {
        let dir = TempDir::new().unwrap();
        let cache = WasmModuleCache::open(dir.path(), TEST_CAP_BYTES).unwrap();
        let (store, addr) = make_store();
        let provider = DiskCachedWasmTemplateProvider::new(store, cache.clone());
        provider.get_template(&addr).unwrap().expect("loaded");
        cache.flush();

        // Damage a shape count, leaving the wasmer artifact itself intact.
        let path = cache.path_for(&addr);
        let mut bytes = fs::read(&path).unwrap();
        bytes[8] ^= 0x01;
        fs::write(&path, &bytes).unwrap();

        assert!(cache.try_load(&addr).is_none(), "a bad integrity tag is a miss");
        assert!(!path.exists(), "the file with the bad tag should be removed");
    }

    // The bytes the header's own tag never reached before it covered the body: the artifact's first
    // byte, a byte in the middle of it, and its last. Each is a byte `deserialize_unchecked` would
    // otherwise have read as native code or as an rkyv offset.
    #[test]
    fn a_flipped_artifact_byte_falls_back_to_recompile() {
        for offset_from in [
            |_len: usize| HEADER_BYTES,
            |len: usize| HEADER_BYTES + (len - HEADER_BYTES) / 2,
            |len: usize| len - 1,
        ] {
            let dir = TempDir::new().unwrap();
            let cache = WasmModuleCache::open(dir.path(), TEST_CAP_BYTES).unwrap();
            let (store, addr) = make_store();
            let provider = DiskCachedWasmTemplateProvider::new(store, cache.clone());
            provider.get_template(&addr).unwrap().expect("loaded");
            cache.flush();

            let path = cache.path_for(&addr);
            let mut bytes = fs::read(&path).unwrap();
            assert!(bytes.len() > HEADER_BYTES, "the artifact body should not be empty");
            let offset = offset_from(bytes.len());
            bytes[offset] ^= 0x01;
            fs::write(&path, &bytes).unwrap();

            assert!(
                cache.try_load(&addr).is_none(),
                "a damaged artifact at {offset} is a miss"
            );
            assert!(!path.exists(), "the damaged file should be removed");
        }
    }

    // A whole artifact lifted from another template's file, under this build's fingerprint. The tag
    // covers the address, so the file only answers for the name it is found under.
    #[test]
    fn an_artifact_under_the_wrong_address_falls_back_to_recompile() {
        let dir = TempDir::new().unwrap();
        let cache = WasmModuleCache::open(dir.path(), TEST_CAP_BYTES).unwrap();
        let (store, addr) = make_store();
        let provider = DiskCachedWasmTemplateProvider::new(store, cache.clone());
        provider.get_template(&addr).unwrap().expect("loaded");
        cache.flush();

        let bytes = fs::read(cache.path_for(&addr)).unwrap();
        let other = TemplateAddress::from_array([9u8; 32]);
        let other_path = cache.path_for(&other);
        fs::write(&other_path, &bytes).unwrap();

        assert!(cache.try_load(&other).is_none());
        assert!(!other_path.exists());
    }

    #[test]
    fn flipped_crc_byte_falls_back_to_recompile() {
        let dir = TempDir::new().unwrap();
        let cache = WasmModuleCache::open(dir.path(), TEST_CAP_BYTES).unwrap();
        let (store, addr) = make_store();
        let provider = DiskCachedWasmTemplateProvider::new(store, cache.clone());
        provider.get_template(&addr).unwrap().expect("loaded");
        cache.flush();

        let path = cache.path_for(&addr);
        let mut bytes = fs::read(&path).unwrap();
        bytes[TAG_OFFSET] ^= 0x01;
        fs::write(&path, &bytes).unwrap();

        assert!(cache.try_load(&addr).is_none());
        assert!(!path.exists());
    }

    #[test]
    fn truncated_header_treated_as_miss() {
        let dir = TempDir::new().unwrap();
        let cache = WasmModuleCache::open(dir.path(), TEST_CAP_BYTES).unwrap();
        let (_store, addr) = make_store();

        let path = cache.path_for(&addr);
        fs::write(&path, vec![0u8; HEADER_BYTES - 1]).unwrap();

        assert!(cache.try_load(&addr).is_none());
        assert!(!path.exists(), "a short file should be removed");
    }

    #[test]
    fn the_stored_tag_covers_the_fields_and_the_body() {
        let dir = TempDir::new().unwrap();
        let cache = WasmModuleCache::open(dir.path(), TEST_CAP_BYTES).unwrap();
        let (store, addr) = make_store();
        let provider = DiskCachedWasmTemplateProvider::new(store, cache.clone());
        let loaded = provider.get_template(&addr).unwrap().expect("loaded");
        cache.flush();

        let bytes = fs::read(cache.path_for(&addr)).unwrap();
        let stored_tag = u64::from_le_bytes(bytes[TAG_OFFSET..HEADER_BYTES].try_into().unwrap());
        assert_eq!(
            stored_tag,
            integrity_tag(&addr, &bytes[..TAG_OFFSET], &bytes[HEADER_BYTES..]),
        );

        let reloaded = cache.try_load(&addr).expect("hit");
        assert_eq!(reloaded.code_size(), loaded.code_size());

        // `instantiation_points` prices each shape count with its own constant, so a hit must serve
        // them in the slots a fresh compile wrote them to.
        let LoadedTemplate::Wasm(reloaded) = &reloaded;
        let LoadedTemplate::Wasm(loaded) = &loaded;
        assert_eq!(reloaded.shape(), loaded.shape());
    }

    #[test]
    fn an_offered_artifact_is_published_by_the_writer() {
        let (loaded, size) = compiled_artifact();
        let dir = TempDir::new().unwrap();
        let cache = WasmModuleCache::open(dir.path(), TEST_CAP_BYTES).unwrap();
        let addr = addr_of_byte(0x11);

        cache.store(&addr, &loaded);
        cache.flush();

        assert!(cache.try_load(&addr).is_some());
        assert_eq!(cache.total_bytes(), size);
    }

    #[test]
    fn the_writer_outlives_a_write_it_cannot_complete() {
        let (loaded, _) = compiled_artifact();
        let dir = TempDir::new().unwrap();
        let cache = WasmModuleCache::open(dir.path(), TEST_CAP_BYTES).unwrap();

        // Nowhere to write, so the first artifact fails somewhere between tempfile and rename.
        fs::remove_dir_all(dir.path()).unwrap();
        cache.store(&addr_of_byte(0x11), &loaded);
        cache.flush();

        fs::create_dir_all(dir.path()).unwrap();
        let addr = addr_of_byte(0x22);
        cache.store(&addr, &loaded);
        cache.flush();

        assert!(
            cache.path_for(&addr).exists(),
            "one artifact that could not be written must not cost every artifact after it",
        );
    }

    #[test]
    fn evicts_the_least_recently_used_artifact_over_the_cap() {
        let (loaded, size) = compiled_artifact();
        let dir = TempDir::new().unwrap();
        // Room for one artifact and change, so the second store must displace the first.
        let cache = WasmModuleCache::open(dir.path(), size + size / 2).unwrap();

        let first = addr_of_byte(0x11);
        let second = addr_of_byte(0x22);
        cache.store(&first, &loaded);
        cache.store(&second, &loaded);
        cache.flush();

        assert!(
            !cache.path_for(&first).exists(),
            "the colder artifact should be evicted"
        );
        assert!(
            cache.path_for(&second).exists(),
            "the artifact just stored should be kept"
        );
        assert_eq!(cache.total_bytes(), size);
    }

    #[test]
    fn a_hit_spares_an_artifact_from_the_next_eviction() {
        let (loaded, size) = compiled_artifact();
        let dir = TempDir::new().unwrap();
        // Room for two artifacts, so the third store evicts whichever went longest unwanted.
        let cache = WasmModuleCache::open(dir.path(), 2 * size + size / 2).unwrap();

        let first = addr_of_byte(0x11);
        let second = addr_of_byte(0x22);
        let third = addr_of_byte(0x33);
        cache.store(&first, &loaded);
        cache.store(&second, &loaded);
        cache.flush();
        cache.try_load(&first).expect("hit");
        cache.store(&third, &loaded);
        cache.flush();

        assert!(cache.path_for(&first).exists(), "the artifact just read should be kept");
        assert!(
            !cache.path_for(&second).exists(),
            "the artifact nobody wanted should go"
        );
        assert!(cache.path_for(&third).exists());
    }

    #[test]
    fn an_artifact_larger_than_the_cap_is_kept() {
        let (loaded, _) = compiled_artifact();
        let dir = TempDir::new().unwrap();
        let cache = WasmModuleCache::open(dir.path(), 1).unwrap();

        let addr = addr_of_byte(0x11);
        cache.store(&addr, &loaded);
        cache.flush();

        assert!(
            cache.path_for(&addr).exists(),
            "evicting the only artifact would recompile it on every call",
        );
    }

    #[test]
    fn reopening_tallies_what_is_already_on_disk() {
        let (loaded, size) = compiled_artifact();
        let dir = TempDir::new().unwrap();
        let first = addr_of_byte(0x11);
        let second = addr_of_byte(0x22);
        {
            let cache = WasmModuleCache::open(dir.path(), TEST_CAP_BYTES).unwrap();
            cache.store(&first, &loaded);
            cache.store(&second, &loaded);
            cache.flush();
        }

        let reopened = WasmModuleCache::open(dir.path(), TEST_CAP_BYTES).unwrap();
        assert_eq!(reopened.total_bytes(), 2 * size);

        // A cap below what the directory holds is applied before the first lookup.
        let shrunk = WasmModuleCache::open(dir.path(), size + size / 2).unwrap();
        assert_eq!(shrunk.total_bytes(), size);
    }

    #[test]
    fn opening_reaps_artifacts_under_a_superseded_fingerprint() {
        let (loaded, size) = compiled_artifact();
        let dir = TempDir::new().unwrap();
        let addr = addr_of_byte(0x11);
        {
            let cache = WasmModuleCache::open(dir.path(), TEST_CAP_BYTES).unwrap();
            cache.store(&addr, &loaded);
            cache.flush();
        }

        // An artifact this build cannot read, left by a run under an older engine config, and a file
        // that is not this cache's at all.
        let superseded = dir.path().join(format!("{}_v0.bin", addr_of_byte(0x22)));
        let unrelated = dir.path().join("notes.txt");
        fs::write(&superseded, b"an older generation").unwrap();
        fs::write(&unrelated, b"not ours").unwrap();

        let cache = WasmModuleCache::open(dir.path(), TEST_CAP_BYTES).unwrap();

        assert!(
            !superseded.exists(),
            "nothing else will ever collect a superseded generation",
        );
        assert!(unrelated.exists(), "a file this cache did not write is left alone");
        assert!(cache.path_for(&addr).exists());
        assert_eq!(cache.total_bytes(), size);
    }

    #[test]
    fn opening_reaps_abandoned_tempfiles_only() {
        let dir = TempDir::new().unwrap();
        let addr = addr_of_byte(0x11);
        let abandoned = dir
            .path()
            .join(format!("{}_{}.bin.tmp.1234.0", addr, ENGINE_FINGERPRINT));
        let in_flight = dir
            .path()
            .join(format!("{}_{}.bin.tmp.5678.0", addr, ENGINE_FINGERPRINT));
        fs::write(&abandoned, b"partial").unwrap();
        fs::write(&in_flight, b"partial").unwrap();

        let aged = SystemTime::now() - (STALE_TEMPFILE_AGE * 2);
        fs::File::options()
            .write(true)
            .open(&abandoned)
            .unwrap()
            .set_times(fs::FileTimes::new().set_accessed(aged).set_modified(aged))
            .unwrap();

        let cache = WasmModuleCache::open(dir.path(), TEST_CAP_BYTES).unwrap();

        assert!(!abandoned.exists(), "a tempfile no store is writing should be reaped");
        assert!(
            in_flight.exists(),
            "a tempfile a store may still be writing should be left"
        );
        assert_eq!(cache.total_bytes(), 0, "tempfiles do not count towards the cap");
    }

    #[test]
    #[cfg(unix)]
    fn a_republished_file_survives_a_stale_unlink() {
        let (loaded, _) = compiled_artifact();
        let dir = TempDir::new().unwrap();
        let cache = WasmModuleCache::open(dir.path(), TEST_CAP_BYTES).unwrap();
        let addr = addr_of_byte(0x11);

        cache.store(&addr, &loaded);
        cache.flush();
        let path = cache.path_for(&addr);
        let stale_identity = fs::metadata(&path).ok().as_ref().and_then(identity_of);

        // A second store publishes a new inode over the same name.
        cache.store(&addr, &loaded);
        cache.flush();

        remove_tracked_file(&path, stale_identity);

        assert!(
            path.exists(),
            "an unlink decided against the replaced file must not land"
        );
        assert!(cache.try_load(&addr).is_some());
    }

    #[test]
    fn fingerprint_mismatch_treated_as_miss() {
        let dir = TempDir::new().unwrap();
        let cache = WasmModuleCache::open(dir.path(), TEST_CAP_BYTES).unwrap();
        let (_store, addr) = make_store();

        // Plant a file under a different fingerprint suffix, after the open that would have reaped it.
        let alt = dir.path().join(format!("{}_v0.bin", addr));
        fs::write(&alt, b"some bytes").unwrap();

        // A lookup names this build's file, which does not exist, and never reads the other.
        assert!(cache.try_load(&addr).is_none());
        assert!(alt.exists());
    }
}
