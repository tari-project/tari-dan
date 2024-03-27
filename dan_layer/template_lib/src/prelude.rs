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

//! The prelude contains all the commonly used types and functions that are used. To use it, add the import `use
//! tari_template_lib::prelude::*;`

#[cfg(all(feature = "macro", target_arch = "wasm32"))]
pub use tari_template_macros::template;
#[cfg(all(feature = "macro", not(target_arch = "wasm32")))]
pub use tari_template_macros::template_non_wasm as template;

pub use crate::{
    args,
    auth::{ComponentAccessRules as AccessRules, RestrictedAccessRule::*, *},
    caller_context::CallerContext,
    component::{Component, ComponentManager},
    consensus::Consensus,
    constants::{CONFIDENTIAL_TARI_RESOURCE_ADDRESS, PUBLIC_IDENTITY_RESOURCE_ADDRESS, XTR2},
    crypto::{PedersonCommitmentBytes, RistrettoPublicKeyBytes},
    debug,
    error,
    events::emit_event,
    info,
    invoke_args,
    log,
    models::{
        AddressAllocation,
        Amount,
        Bucket,
        BucketId,
        ComponentAddress,
        ConfidentialOutputProof,
        ConfidentialWithdrawProof,
        Metadata,
        NonFungible,
        NonFungibleAddress,
        NonFungibleId,
        Proof,
        ProofId,
        ResourceAddress,
        TemplateAddress,
        Vault,
        VaultId,
    },
    rand,
    resource::{ResourceBuilder, ResourceManager, ResourceType},
    template::{BuiltinTemplate, TemplateManager},
    warn,
};
