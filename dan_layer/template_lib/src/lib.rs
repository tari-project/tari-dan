//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

//! This crate contains an interface for WASM templates to interact with the state of the Tari Network, as well as
//! some utilities for executing functions that may be slow in the WASM environment.
//!
//! In most cases, you will only require the `prelude` which can be included with:
//! ```
//! use tari_template_lib::prelude::*;
//! ```

pub mod auth;

mod hash;
pub use hash::{Hash, HashParseError};

#[macro_use]
pub mod args;
pub mod models;

pub mod component;
mod consensus;
pub use consensus::Consensus;

pub mod caller_context;
mod context;
pub use context::{get_context, init_context, AbiContext};

pub mod rand;
pub mod resource;

pub mod crypto;
pub mod events;

pub mod template;

// ---------------------------------------- WASM target exports ------------------------------------------------

#[cfg(target_arch = "wasm32")]
pub mod template_dependencies;

mod engine;
pub use engine::engine;

#[cfg(target_arch = "wasm32")]
pub mod panic_hook;
pub mod prelude;
#[cfg(feature = "macro")]
pub use prelude::template;
// Re-export for macro
pub use tari_bor::encode;

pub mod constants;
#[cfg(target_arch = "wasm32")]
pub mod workspace;

#[macro_use]
mod newtype_serde_macros;
