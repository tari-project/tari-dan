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

use tari_template_abi::{call_engine, EngineOp};

use crate::{
    args::{InvokeResult, MintResourceArg, ResourceAction, ResourceInvokeArg, ResourceRef},
    models::{Bucket, Metadata, ResourceAddress},
};

#[derive(Debug)]
pub struct ResourceManager {
    for_specific: Option<ResourceAddress>,
}

impl ResourceManager {
    pub(crate) fn new() -> Self {
        ResourceManager { for_specific: None }
    }

    pub fn get(address: ResourceAddress) -> Self {
        ResourceManager {
            for_specific: Some(address),
        }
    }

    pub(super) fn mint_resource(&mut self, arg: MintResourceArg) -> Bucket {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: if let Some(resource_adddress) = self.for_specific {
                ResourceRef::Ref(resource_adddress)
            } else {
                ResourceRef::Resource
            },
            action: ResourceAction::Mint,
            args: invoke_args![arg],
        });

        let resource_address = resp.decode().expect("Failed to decode Bucket");
        Bucket::new(resource_address)
    }

    // Register, but don't mind any tokens
    pub fn register_non_fungible(&mut self, metadata: Metadata) -> ResourceAddress {
        let arg = MintResourceArg::NonFungible {
            resource_address: None,
            token_ids: vec![],
            metadata,
        };
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: if let Some(resource_adddress) = self.for_specific {
                ResourceRef::Ref(resource_adddress)
            } else {
                ResourceRef::Resource
            },
            action: ResourceAction::Mint,
            args: invoke_args![arg],
        });

        resp.decode().expect("Failed to decode ResourceAddress")
    }

    pub fn mint_non_fungible(&mut self, name: &str, image_url: &str, ids: Vec<u64>) -> Bucket {
        let mut metadata = Metadata::new();
        metadata.insert(b"NAME".to_vec(), name.as_bytes().to_vec());
        metadata.insert(b"IMAGE_URL".to_vec(), image_url.as_bytes().to_vec());

        let arg = MintResourceArg::NonFungible {
            resource_address: self.for_specific,
            token_ids: ids,
            metadata,
        };
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: if let Some(resource_adddress) = self.for_specific {
                ResourceRef::Ref(resource_adddress)
            } else {
                ResourceRef::Resource
            },
            action: ResourceAction::Mint,
            args: invoke_args![arg],
        });

        let resource_address = resp.decode().expect("Failed to decode Bucket");
        Bucket::new(resource_address)
    }
}
