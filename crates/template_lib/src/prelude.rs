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

pub use tari_bor;
#[expect(deprecated)]
pub use tari_template_lib_types::constants::XTR;
#[cfg(feature = "extra-maps")]
pub use tari_template_lib_types::fast_hash::{FastMap, PrehashedMap, indexmap_codec};
pub use tari_template_lib_types::{
    AccessRule,
    AuthHook,
    AuthHookCaller,
    ComponentAddress,
    Hash32,
    Hash64,
    MaxBytes,
    MaxString,
    Metadata,
    NonFungibleAddress,
    NonFungibleId,
    OwnerRule,
    ResourceAddress,
    ResourceType,
    TemplateAddress,
    UtxoAddress,
    UtxoId,
    VaultId,
    access_rules::{
        ComponentAccessRules as AccessRules,
        LOCKED,
        OWNER,
        ResourceAuthAction,
        RestrictedAccessRule::*,
        UpdateRule,
        *,
    },
    bytes::Bytes,
    confidential::{ConfidentialOutputStatement, ConfidentialWithdrawProof},
    constants::{PUBLIC_IDENTITY_RESOURCE_ADDRESS, STEALTH_TARI_RESOURCE_ADDRESS, TARI_TOKEN},
    crypto::{
        BalanceProofSignature,
        PedersenCommitmentBytes,
        PublicKey,
        RistrettoPublicKeyBytes,
        Scalar32Bytes,
        SchnorrSignatureBytes,
        Signature,
        SignatureDomain,
        SignaturePayload,
    },
    custom_signature_domain,
    metadata,
    rule,
    stealth::{
        CurrentInputView,
        MerkleProof,
        SpendAuthorization,
        SpendCondition,
        SpendWitness,
        StealthInputView,
        StealthInputsStatement,
        StealthOutputView,
        StealthOutputsStatement,
        StealthTransferStatement,
        TemplateFunction,
    },
};
#[cfg(all(feature = "macro", target_arch = "wasm32"))]
pub use tari_template_macros::template;
#[cfg(all(feature = "macro", not(target_arch = "wasm32")))]
pub use tari_template_macros::template_non_wasm as template;

/// Native primitives — Ristretto arithmetic, hashing, signature verification. Exposed as a module
/// rather than glob-imported: these are specialist and read better qualified as `intrinsics::…`.
pub use crate::intrinsics;
pub use crate::{
    args::{VaultFreezeFlag, VaultFreezeFlags},
    caller_context::CallerContext,
    component::{Component, ComponentManager},
    consensus::Consensus,
    debug,
    error,
    events::emit_event,
    info,
    invoke_args as args,
    log,
    models::{
        Account,
        Bucket,
        BucketId,
        ComponentAddressAllocation,
        NonFungible,
        Proof,
        ProofId,
        ResourceAddressAllocation,
        SignatureVerifier,
        Vault,
        Verifiable,
    },
    rand,
    resource::{ResourceBuilder, ResourceManager},
    spend_context::SpendContext,
    template::{BuiltinTemplate, TemplateManager},
    types,
    types::{Amount, amount, crypto},
    warn,
};
