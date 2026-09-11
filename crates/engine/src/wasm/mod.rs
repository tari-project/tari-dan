// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

mod error;
pub use error::*;

mod environment;

mod module;
pub use module::{LoadedWasmTemplate, ModuleShape, WasmModule};

mod static_template_def;
pub use static_template_def::{ExtractTemplateDefError, extract_template_def};

#[cfg(feature = "wasm-cache")]
mod cache;
#[cfg(feature = "wasm-cache")]
pub use cache::{
    DiskCachedWasmTemplateProvider,
    DiskCachedWasmTemplateProviderError,
    ENGINE_FINGERPRINT,
    WasmModuleCache,
};

mod metering;
mod process;

pub use process::WasmProcess;

mod limiting_tunable;
mod mem_writer;
