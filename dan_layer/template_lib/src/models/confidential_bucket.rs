//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use tari_bor::{borsh, Decode, Encode};
use tari_template_abi::{call_engine, EngineOp};

use crate::{
    args::{ConfidentialBucketAction, ConfidentialBucketInvokeArg, ConfidentialBucketRef, InvokeResult},
    models::ResourceAddress,
};

pub type ConfidentialBucketId = u32;

#[derive(Debug, Clone, Decode, Encode)]
pub struct ConfidentialBucket {
    id: ConfidentialBucketId,
}

impl ConfidentialBucket {
    pub fn new(id: ConfidentialBucketId) -> Self {
        Self { id }
    }

    pub fn id(&self) -> ConfidentialBucketId {
        self.id
    }

    pub fn resource_address(&self) -> ResourceAddress {
        let resp: InvokeResult = call_engine(EngineOp::ConfidentialBucketInvoke, &ConfidentialBucketInvokeArg {
            bucket_ref: ConfidentialBucketRef::Ref(self.id),
            action: ConfidentialBucketAction::GetResourceAddress,
            args: invoke_args![],
        });

        resp.decode()
            .expect("Confidential Bucket GetResourceAddress returned invalid resource address")
    }
}
