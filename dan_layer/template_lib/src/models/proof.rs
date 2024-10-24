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

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use tari_bor::BorTag;
use tari_template_abi::{call_engine, rust::fmt, EngineOp};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::{
    args::{InvokeResult, ProofAction, ProofInvokeArg, ProofRef},
    models::{Amount, BinaryTag, NonFungibleId, ResourceAddress},
    prelude::ResourceType,
};

const TAG: u64 = BinaryTag::ProofId.as_u64();

/// The unique identification of a proof during a transaction execution
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Ord, PartialOrd, Hash)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ProofId(#[cfg_attr(feature = "ts", ts(type = "number"))] BorTag<u32, TAG>);

impl From<u32> for ProofId {
    fn from(value: u32) -> Self {
        Self(BorTag::new(value))
    }
}

impl fmt::Display for ProofId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ProofId({})", self.0.inner())
    }
}

/// Allows a user to prove ownership of a resource. Proofs only live during the execution of a transaction.
/// The main use case is to prove that the user has a specific badge during cross-template calls
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Proof {
    id: ProofId,
}

impl Proof {
    pub const fn from_id(id: ProofId) -> Self {
        Self { id }
    }

    pub fn id(&self) -> ProofId {
        self.id
    }

    pub fn resource_address(&self) -> ResourceAddress {
        let resp: InvokeResult = call_engine(EngineOp::ProofInvoke, &ProofInvokeArg {
            proof_ref: ProofRef::Ref(self.id),
            action: ProofAction::GetResourceAddress,
            args: invoke_args![],
        });

        resp.decode()
            .expect("Proof GetResourceAddress returned invalid resource address")
    }

    pub fn resource_type(&self) -> ResourceType {
        let resp: InvokeResult = call_engine(EngineOp::ProofInvoke, &ProofInvokeArg {
            proof_ref: ProofRef::Ref(self.id),
            action: ProofAction::GetResourceType,
            args: invoke_args![],
        });

        resp.decode()
            .expect("Proof GetResourceType returned invalid resource type")
    }

    pub fn get_non_fungibles(&self) -> BTreeSet<NonFungibleId> {
        let resp: InvokeResult = call_engine(EngineOp::ProofInvoke, &ProofInvokeArg {
            proof_ref: ProofRef::Ref(self.id),
            action: ProofAction::GetNonFungibles,
            args: invoke_args![],
        });

        resp.decode()
            .expect("Proof GetNonFungibles returned invalid non-fungibles")
    }

    pub fn amount(&self) -> Amount {
        let resp: InvokeResult = call_engine(EngineOp::ProofInvoke, &ProofInvokeArg {
            proof_ref: ProofRef::Ref(self.id),
            action: ProofAction::GetAmount,
            args: invoke_args![],
        });

        resp.decode().expect("Proof GetAmount returned invalid amount")
    }

    #[must_use = "ProofAccess must used"]
    pub fn authorize(&self) -> ProofAccess {
        self.try_authorize().expect("Proof authorization failed")
    }

    pub fn authorize_with<F: FnOnce() -> R, R>(&self, f: F) -> R {
        let _auth = self.try_authorize().expect("Proof authorization failed");
        f()
    }

    /// Try to authorize the proof. If the proof cannot be authorized, this will return an error.
    pub fn try_authorize(&self) -> Result<ProofAccess, NotAuthorized> {
        let resp: InvokeResult = call_engine(EngineOp::ProofInvoke, &ProofInvokeArg {
            proof_ref: ProofRef::Ref(self.id),
            action: ProofAction::Authorize,
            args: invoke_args![],
        });

        resp.decode::<Result<(), NotAuthorized>>()
            .expect("Proof Access error")?;
        Ok(ProofAccess { id: self.id })
    }

    /// Drop/destroy this proof
    pub fn drop(self) {
        let resp: InvokeResult = call_engine(EngineOp::ProofInvoke, &ProofInvokeArg {
            proof_ref: ProofRef::Ref(self.id),
            action: ProofAction::Drop,
            args: invoke_args![],
        });

        resp.decode().expect("Proof drop error")
    }

    pub fn assert_resource(&self, resource_address: ResourceAddress) {
        assert_eq!(
            self.resource_address(),
            resource_address,
            "Proof of resource did not match {resource_address}"
        );
    }
}

/// Returned when a proof cannot be authorized
#[derive(Debug, Serialize, Deserialize)]
pub struct NotAuthorized;

/// TODO: Clean this up
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofAccess {
    pub id: ProofId,
}

impl Drop for ProofAccess {
    fn drop(&mut self) {
        let resp: InvokeResult = call_engine(EngineOp::ProofInvoke, &ProofInvokeArg {
            proof_ref: ProofRef::Ref(self.id),
            action: ProofAction::DropAuthorize,
            args: invoke_args![],
        });

        resp.decode::<()>()
            .unwrap_or_else(|_| panic!("Drop failed for proof {}", self.id));
    }
}

/// A convenience wrapper for managing proofs in templates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofAuth {
    pub id: ProofId,
}

impl Drop for ProofAuth {
    fn drop(&mut self) {
        let resp: InvokeResult = call_engine(EngineOp::ProofInvoke, &ProofInvokeArg {
            proof_ref: ProofRef::Ref(self.id),
            action: ProofAction::Drop,
            args: invoke_args![],
        });

        resp.decode::<()>()
            .unwrap_or_else(|_| panic!("Drop failed for proof {}", self.id));
    }
}
