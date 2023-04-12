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

use serde::Serialize;
use tari_bor::encode;
use tari_template_abi::{call_engine, rust::collections::HashMap, EngineOp};

use crate::{
    args::{
        CreateResourceArg,
        InvokeResult,
        MintArg,
        MintResourceArg,
        ResourceAction,
        ResourceGetNonFungibleArg,
        ResourceInvokeArg,
        ResourceRef,
        ResourceUpdateNonFungibleDataArg,
    },
    models::{Amount, Bucket, Metadata, NonFungible, NonFungibleId, ResourceAddress},
    prelude::ResourceType,
};

#[derive(Debug)]
pub struct ResourceManager {
    resource_address: Option<ResourceAddress>,
}

impl ResourceManager {
    pub(crate) fn new() -> Self {
        ResourceManager { resource_address: None }
    }

    pub fn get(address: ResourceAddress) -> Self {
        Self {
            resource_address: Some(address),
        }
    }

    fn expect_resource_address(&self) -> ResourceRef {
        let resource_address = self
            .resource_address
            .as_ref()
            .copied()
            .expect("Resource address not set");
        ResourceRef::Ref(resource_address)
    }

    pub fn resource_type(&self) -> ResourceType {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.expect_resource_address(),
            action: ResourceAction::GetResourceType,
            args: invoke_args![],
        });
        resp.decode()
            .expect("Resource GetResourceType returned invalid resource type")
    }

    pub fn create(
        &mut self,
        resource_type: ResourceType,
        token_symbol: String,
        metadata: Metadata,
        mint_arg: Option<MintArg>,
    ) -> (ResourceAddress, Option<Bucket>) {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: ResourceRef::Resource,
            action: ResourceAction::Create,
            args: invoke_args![CreateResourceArg {
                resource_type,
                token_symbol,
                metadata,
                mint_arg
            }],
        });

        resp.decode()
            .expect("[register_non_fungible] Failed to decode ResourceAddress")
    }

    pub fn mint_non_fungible<T: Serialize, U: Serialize>(
        &mut self,
        id: NonFungibleId,
        metadata: &T,
        mutable_data: &U,
    ) -> Bucket {
        self.mint_internal(MintResourceArg {
            mint_arg: MintArg::NonFungible {
                tokens: Some((id, (encode(metadata).unwrap(), encode(mutable_data).unwrap())))
                    .into_iter()
                    .collect(),
            },
        })
    }

    pub fn mint_many_non_fungible<T: Serialize, U: Serialize>(
        &mut self,
        metadata: &T,
        mutable_data: &U,
        supply: usize,
    ) -> Bucket {
        let mut tokens: HashMap<NonFungibleId, (Vec<u8>, Vec<u8>)> = HashMap::with_capacity(supply);
        let token_data = (encode(metadata).unwrap(), encode(mutable_data).unwrap());
        for _ in 0..supply {
            let id = NonFungibleId::random();
            tokens.insert(id, token_data.clone());
        }
        self.mint_internal(MintResourceArg {
            mint_arg: MintArg::NonFungible { tokens },
        })
    }

    pub fn mint_fungible(&mut self, amount: Amount) -> Bucket {
        self.mint_internal(MintResourceArg {
            mint_arg: MintArg::Fungible { amount },
        })
    }

    fn mint_internal(&mut self, arg: MintResourceArg) -> Bucket {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.expect_resource_address(),
            action: ResourceAction::Mint,
            args: invoke_args![arg],
        });

        let bucket_id = resp.decode().expect("Failed to decode Bucket");
        Bucket::from_id(bucket_id)
    }

    pub fn total_supply(&self) -> Amount {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.expect_resource_address(),
            action: ResourceAction::GetTotalSupply,
            args: invoke_args![],
        });

        resp.decode().expect("[total_supply] Failed to decode Amount")
    }

    pub fn get_non_fungible(&self, id: &NonFungibleId) -> NonFungible {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.expect_resource_address(),
            action: ResourceAction::GetNonFungible,
            args: invoke_args![ResourceGetNonFungibleArg { id: id.clone() }],
        });

        resp.decode().expect("[get_non_fungible] Failed to decode NonFungible")
    }

    pub fn update_non_fungible_data<T: Serialize + ?Sized>(&self, id: NonFungibleId, data: &T) {
        let resp: InvokeResult = call_engine(EngineOp::ResourceInvoke, &ResourceInvokeArg {
            resource_ref: self.expect_resource_address(),
            action: ResourceAction::UpdateNonFungibleData,
            args: invoke_args![ResourceUpdateNonFungibleDataArg {
                id,
                data: encode(data).unwrap()
            }],
        });

        resp.decode()
            .expect("[update_non_fungible_data] Failed to decode Amount")
    }
}
