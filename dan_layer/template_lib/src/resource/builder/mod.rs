//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

mod confidential;
mod fungible;
mod non_fungible;

use crate::resource::builder::{
    confidential::ConfidentialResourceBuilder,
    fungible::FungibleResourceBuilder,
    non_fungible::NonFungibleResourceBuilder,
};

/// Metadata key used as convention to represent the symbol (a.k.a. ticker) of a token. Meant as a shorthand,
/// user-friendly identification of the underlying token
pub const TOKEN_SYMBOL: &str = "SYMBOL";

/// Utility for building resources inside templates
pub struct ResourceBuilder;

impl ResourceBuilder {
    /// Returns a new fungible resource builder
    pub fn fungible() -> FungibleResourceBuilder {
        FungibleResourceBuilder::new()
    }

    /// Returns a new non-fungible resource builder
    pub fn non_fungible() -> NonFungibleResourceBuilder {
        NonFungibleResourceBuilder::new()
    }

    /// Returns a new confidential resource builder
    pub fn confidential() -> ConfidentialResourceBuilder {
        ConfidentialResourceBuilder::new()
    }
}
