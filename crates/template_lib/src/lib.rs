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

//! This crate provides ergonomic abstractions that allow WASM templates to interact with the Tari Ootle engine.
//! Almost all Ootle templates written in Rust should depend on this crate.
//!
//! Include the template `prelude` which provides all the necessary imports for writing templates. This includes common
//! types (e.g. `Component`, `Resource`, `Vault`, `AccessRule` etc.) and macros (e.g. the `template` macro).
//! ```
//! use tari_template_lib::prelude::*;
//! ```
//!
//! ## Getting started
//!
//! Include this crate in your `Cargo.toml`:
//! ```toml
//! [dependencies]
//! tari_template_lib = { version = "*" } # Or specify a specific version
//! ```
//!
//! Apply the `template` macro and define your template struct and its methods.
//! This macro generates the necessary ABI functions and boilerplate to make your template work with the Ootle engine.
//!
//! ```rust
//! use tari_template_lib::prelude::*;
//!
//! // The `template` macro generates and implements the necessary ABI functions, `TemplateDefinition` and other boilerplate that
//! // hooks up the functions and methods you define to the Ootle execution engine.
//! // The module name is not important (it is ignored). The `template` macro must be applied to the module that contains your template struct/enum.
//! #[template]
//! mod my_template {
//!    // Bring prelude and anything else into scope.
//!    use super::*;
//!
//!    /// Defines the component state that is stored on-chain. This can be a struct or an enum.
//!    /// The struct/enum name can be selected arbitrarily, and is exposed in the TemplateDefinition for the template.
//!    pub struct MyCounter {
//!       counter: u64,
//!       /// Vaults contain resources. A vault is a separate substate created within a component, and belongs to that component.
//!       vault: Vault,
//!    }
//!
//!    impl MyCounter {
//!      /// A simple constructor.
//!      /// NOTE: this is a convenience constructor that implicitly creates a component with some defaults. These defaults are restrictive for many use cases
//!      /// for e.g. all component methods are only callable by the component owner (i.e. the same signer that created the component must sign the transaction
//!      /// to call any method).
//!      /// To see how to customise this, see the `custom` constructor below.
//!      pub fn new() -> Self {
//!          Self {
//!             counter: 0,
//!             // A empty vault that can only hold the native token (TARI). You can also create vaults that hold your own resources (see the `ResourceBuilder`).
//!             vault: Vault::new_empty(TARI_TOKEN),
//!          }
//!      }
//!
//!      /// This constructor provides more control over the component configuration.
//!      ///
//!      /// ## Arguments
//!      /// - `address`: An component address allocation. This allows a single transaction to both create and call onto the created component without knowing the component address in advance.
//!      /// - `access_rules`: Custom method access rules for the component. All methods referenced in the access rules must be defined on the component or the transaction will fail.
//!      /// - `owner_rule`: Custom owner rule for the component. The owner of a component is able to change access rules and call all methods. Setting `OwnerRule::None` means the component has no owner and all access rules are immutable.
//!      pub fn custom(address: ComponentAddressAllocation, access_rules: ComponentAccessRules, owner_rule: OwnerRule) -> Component<Self> {
//!        // Call `new` which initialises the component state. NOTE that this is a completely normal function call. The component
//!        // is not yet created on-chain.
//!        let component = Self::new();
//!
//!        Component::new(component)
//!           .with_address_allocation(address)
//!           .with_access_rules(access_rules)
//!           .with_owner_rule(owner_rule)
//!           // Create the component on-chain
//!           .create()
//!      }
//!
//!     /// Increment the counter by 1. The `CallMethod` instruction is used to invoke this method.
//!     /// This method is callable as per the access rules defined for the component.
//!     pub fn increment(&mut self) {
//!       self.counter += 1;
//!     }
//!
//!     /// A simple associated function that returns a string. The `CallFunction` instruction is used to invoke this function.
//!     /// Apart from defining constructors, associated functions can provide any shared functionality callable by anyone.
//!     pub fn some_function(name: String, number: u64) -> String {
//!        format!("Hello {name}, the number is {number}")
//!     }
//!   }
//! }
//! ```
//!
//! ## Customising the CBOR wire format
//!
//! By default the `template` macro injects `#[derive(minicbor::Encode, minicbor::Decode,
//! minicbor::CborLen)]` and assigns sequential `#[n(N)]` indices to every field/variant of every
//! struct and enum it sees. Two override knobs are available:
//!
//! - **Per-field**: write `#[n(N)]` (or `#[b(N)]`, `#[cbor(n(N))]`) on the field directly. The macro detects an
//!   existing index and leaves the field alone. Useful for pinning an individual field to a stable index while letting
//!   the rest auto-number.
//! - **Per-template**: pass `skip_cbor_derives` to the macro, i.e. `#[template(skip_cbor_derives)]`. The macro will not
//!   inject the `minicbor` derives or any `#[n(N)]` tags. The author is then responsible for writing the derives and
//!   full index assignment themselves. Useful for templates that need a non-default wire format (e.g. preserving an
//!   existing format across field reorders, or using `#[cbor(map)]` instead of the default array encoding).
//!
//! See the [`tari_template_macros`] crate docs for examples.
//!
//! ## Template Examples
//!
//! - <https://github.com/tari-project/wasm-examples>
//! - <https://github.com/tari-project/tari-ootle/tree/development/crates/engine/tests/templates>
//!
//! ## Re-exports
//!
//! This crate re-exports common types (`tari_template_lib_types`), low-level ABI functions in `tari_template_abi` and
//! the `tari_template_macros` proc macro.
//!
//! ## no_std
//!
//! This crate supports `no_std` environments. To use in `no_std`, disable the `std` feature (`default-features =
//! false`) and enable the `alloc` feature.
//!
//! ```toml
//! [dependencies]
//! tari_template_lib = { version = "*", default-features = false, features = ["alloc"] }
//! ```
//!
//! You will need to provide a global allocator (e.g. `talc`, `lol_alloc`). This crate provides a panic handler for
//! `no_std`.

// Support no_std environments
#![cfg_attr(not(feature = "std"), no_std)]

// This can be uncommented if you need to check for mistaken use of the std crate
// TODO: to always use this, we'd need to include the rust prelude where ever ts_rs is used.
// #![no_std]
// #[cfg(feature = "std")]
// extern crate std;

// Some helpful compile-time messages to ensure that the crate is used with either `std` or `alloc` when targeting
// wasm32, but not both at the same time.
#[cfg(all(target_arch = "wasm32", not(any(feature = "std", feature = "alloc"))))]
compile_error!("Either feature `std` or `alloc` must be enabled for this crate.");
#[cfg(all(target_arch = "wasm32", feature = "std", feature = "alloc"))]
compile_error!("Feature `std` and `alloc` can't be enabled at the same time.");

#[macro_use]
pub mod args;
#[macro_use]
pub mod models;

pub mod component;
mod consensus;
pub use consensus::Consensus;

mod spend_context;
pub use spend_context::SpendContext;

pub mod caller_context;

pub mod rand;
pub mod resource;

pub mod events;

pub mod intrinsics;

pub mod template;

pub use tari_template_lib_types as types;

// ---------------------------------------- WASM exports ------------------------------------------------

pub mod template_macro_deps;

mod engine;
pub use engine::engine;

pub mod panic_hook;
pub mod prelude;
// Re-export for macro
#[cfg(feature = "serde")]
pub use tari_bor::serde;
pub use tari_bor::to_value;

#[macro_use]
pub mod macros;
mod error_variants;
