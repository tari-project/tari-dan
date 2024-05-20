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

use serde::{Deserialize, Serialize};
use tari_bor::BorTag;
use tari_template_abi::{call_engine, rust::fmt, EngineOp};
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::{NonFungible, NonFungibleId};
use crate::{
    args::{BucketAction, BucketInvokeArg, BucketRef, InvokeResult},
    models::{Amount, BinaryTag, ConfidentialWithdrawProof, Proof, ResourceAddress},
    prelude::ResourceType,
};

const TAG: u64 = BinaryTag::BucketId.as_u64();

/// A bucket's unique identification during the transaction execution
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Ord, PartialOrd, Hash)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct BucketId(#[cfg_attr(feature = "ts", ts(type = "number"))] BorTag<u32, TAG>);

impl From<u32> for BucketId {
    fn from(value: u32) -> Self {
        Self(BorTag::new(value))
    }
}

impl fmt::Display for BucketId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BucketId({})", self.0.inner())
    }
}

/// A temporary container of resources. Buckets only live during a transaction execution and must be empty at the end of
/// the transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Bucket {
    id: BucketId,
}

impl Bucket {
    pub const fn from_id(id: BucketId) -> Self {
        Self { id }
    }

    pub fn id(&self) -> BucketId {
        self.id
    }

    /// Returns the resource address of the tokens that this bucket holds
    pub fn resource_address(&self) -> ResourceAddress {
        let resp: InvokeResult = call_engine(EngineOp::BucketInvoke, &BucketInvokeArg {
            bucket_ref: BucketRef::Ref(self.id),
            action: BucketAction::GetResourceAddress,
            args: invoke_args![],
        });

        resp.decode()
            .expect("Bucket GetResourceAddress returned invalid resource address")
    }

    /// Returns the the type of resource that this bucket holds
    pub fn resource_type(&self) -> ResourceType {
        let resp: InvokeResult = call_engine(EngineOp::BucketInvoke, &BucketInvokeArg {
            bucket_ref: BucketRef::Ref(self.id),
            action: BucketAction::GetResourceType,
            args: invoke_args![],
        });

        resp.decode()
            .expect("Bucket GetResourceType returned invalid resource type")
    }

    /// Withdraws `amount` tokens from the bucket into a new bucket.
    /// It will panic if there are not enough tokens in the bucket
    pub fn take(&mut self, amount: Amount) -> Self {
        assert!(!amount.is_zero() && amount.is_positive());
        let resp: InvokeResult = call_engine(EngineOp::BucketInvoke, &BucketInvokeArg {
            bucket_ref: BucketRef::Ref(self.id),
            action: BucketAction::Take,
            args: invoke_args![amount],
        });

        resp.decode().expect("Bucket Take returned invalid bucket")
    }

    /// Withdraws an amount (specified in the `proof`) of confidential tokens from the bucket into a new bucket.
    /// It will panic if the proof is invalid or there are not enough tokens in the bucket
    pub fn take_confidential(&mut self, proof: ConfidentialWithdrawProof) -> Self {
        let resp: InvokeResult = call_engine(EngineOp::BucketInvoke, &BucketInvokeArg {
            bucket_ref: BucketRef::Ref(self.id),
            action: BucketAction::TakeConfidential,
            args: invoke_args![proof],
        });

        resp.decode().expect("Bucket Take returned invalid bucket")
    }

    /// Destroy all the tokens that this bucket holds.
    /// It will panic if the caller does not have the appropriate permissions
    pub fn burn(&self) {
        let resp: InvokeResult = call_engine(EngineOp::BucketInvoke, &BucketInvokeArg {
            bucket_ref: BucketRef::Ref(self.id),
            action: BucketAction::Burn,
            args: invoke_args![],
        });

        resp.decode().expect("Bucket Burn returned invalid result")
    }

    /// Split the current bucket, returning two new buckets, one with `amount` tokens and the other with the rest.
    /// It will panic if there are not enough tokens in the bucket
    pub fn split(mut self, amount: Amount) -> (Self, Self) {
        let new_bucket = self.take(amount);
        (new_bucket, self)
    }

    /// Split the current bucket, returning two new buckets, one with an amount (specified in the `proof`) of
    /// confidential tokens and the other with the rest. It will panic if the proof is invalid or there are not
    /// enough tokens in the bucket
    pub fn split_confidential(mut self, proof: ConfidentialWithdrawProof) -> (Self, Self) {
        let new_bucket = self.take_confidential(proof);
        (new_bucket, self)
    }

    /// Returns how many tokens this bucket holds
    pub fn amount(&self) -> Amount {
        let resp: InvokeResult = call_engine(EngineOp::BucketInvoke, &BucketInvokeArg {
            bucket_ref: BucketRef::Ref(self.id),
            action: BucketAction::GetAmount,
            args: invoke_args![],
        });

        resp.decode().expect("Bucket GetAmount returned invalid amount")
    }

    /// Returns a new bucket with revealed funds, specified by the `proof`.
    /// The amount of tokens will not change, only how many of those tokens will be known by everyone
    pub fn reveal_confidential(&mut self, proof: ConfidentialWithdrawProof) -> Bucket {
        let resp: InvokeResult = call_engine(EngineOp::BucketInvoke, &BucketInvokeArg {
            bucket_ref: BucketRef::Ref(self.id),
            action: BucketAction::RevealConfidential,
            args: invoke_args![proof],
        });

        resp.decode()
            .expect("Bucket RevealConfidential returned invalid result")
    }

    /// Create a proof of token balances in the bucket, used mainly for cross-template calls
    pub fn create_proof(&self) -> Proof {
        let resp: InvokeResult = call_engine(EngineOp::BucketInvoke, &BucketInvokeArg {
            bucket_ref: BucketRef::Ref(self.id),
            action: BucketAction::CreateProof,
            args: invoke_args![],
        });

        resp.decode().expect("Bucket CreateProof returned invalid proof")
    }

    /// Returns the IDs of all the non-fungibles in this bucket
    pub fn get_non_fungible_ids(&self) -> Vec<NonFungibleId> {
        let resp: InvokeResult = call_engine(EngineOp::BucketInvoke, &BucketInvokeArg {
            bucket_ref: BucketRef::Ref(self.id),
            action: BucketAction::GetNonFungibleIds,
            args: invoke_args![],
        });

        resp.decode()
            .expect("get_non_fungible_ids returned invalid non fungible ids")
    }

    /// Returns all the non-fungibles in this bucket
    pub fn get_non_fungibles(&self) -> Vec<NonFungible> {
        let resp: InvokeResult = call_engine(EngineOp::BucketInvoke, &BucketInvokeArg {
            bucket_ref: BucketRef::Ref(self.id),
            action: BucketAction::GetNonFungibles,
            args: invoke_args![],
        });

        resp.decode().expect("get_non_fungibles returned invalid non fungibles")
    }

    pub fn count_confidential_commitments(&self) -> u32 {
        let resp: InvokeResult = call_engine(EngineOp::BucketInvoke, &BucketInvokeArg {
            bucket_ref: BucketRef::Ref(self.id),
            action: BucketAction::CountConfidentialCommitments,
            args: invoke_args![],
        });

        resp.decode()
            .expect("count_confidential_commitments returned invalid u32")
    }

    pub fn assert_contains_no_confidential_funds(&self) {
        let count = self.count_confidential_commitments();
        assert_eq!(
            count, 0,
            "Expected bucket to have no confidential commitments, but found {count}",
        );
    }
}
