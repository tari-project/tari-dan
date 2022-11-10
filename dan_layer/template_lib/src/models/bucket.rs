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

use tari_template_abi::{call_engine, Decode, Encode, EngineOp};

use crate::{
    args::{BucketAction, BucketInvokeArg, BucketRef, InvokeResult},
    models::{Amount, ResourceAddress},
};

pub type BucketId = u32;

#[derive(Debug, Clone, Decode, Encode)]
pub struct Bucket {
    id: BucketId,
}

impl Bucket {
    pub(crate) fn new(resource_addr: ResourceAddress) -> Self {
        let resp: InvokeResult = call_engine(EngineOp::BucketInvoke, &BucketInvokeArg {
            bucket_ref: BucketRef::Bucket(resource_addr),
            action: BucketAction::Create,
            args: invoke_args![],
        })
        .expect("Create bucket returned null");

        // TODO: Create bucket with the given resource and get the id
        Self {
            id: resp.decode().expect("Create bucket returned invalid bucket id"),
        }
    }

    pub fn id(&self) -> BucketId {
        self.id
    }

    pub fn resource_address(&self) -> ResourceAddress {
        let resp: InvokeResult = call_engine(EngineOp::BucketInvoke, &BucketInvokeArg {
            bucket_ref: BucketRef::Ref(self.id),
            action: BucketAction::GetResourceAddress,
            args: invoke_args![],
        })
        .expect("Bucket GetResource returned null");

        resp.decode()
            .expect("Bucket GetResource returned invalid resource address")
    }

    pub fn take(&mut self, amount: Amount) -> Self {
        assert!(!amount.is_zero() && amount.is_positive());
        let resp: InvokeResult = call_engine(EngineOp::BucketInvoke, &BucketInvokeArg {
            bucket_ref: BucketRef::Ref(self.id),
            action: BucketAction::Take,
            args: invoke_args![amount],
        })
        .expect("Bucket Take returned null");

        resp.decode().expect("Bucket Take returned invalid bucket")
    }

    pub fn split(mut self, amount: Amount) -> (Self, Self) {
        let new_bucket = self.take(amount);
        (new_bucket, self)
    }
}
