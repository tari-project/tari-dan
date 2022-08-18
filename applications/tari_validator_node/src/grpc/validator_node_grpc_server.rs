// Copyright 2021. The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::convert::TryInto;

use tari_common_types::types::PublicKey;
use tari_comms::NodeIdentity;
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_core::{
    services::{AssetProcessor, AssetProxy, ServiceSpecification},
    storage::DbFactory,
};
use tari_dan_engine::instructions::Instruction;
use tari_vn_grpc::tari_rpc::{
    validator_node_server::ValidatorNode,
    Authority,
    GetIdentityRequest,
    GetIdentityResponse,
    InvokeMethodRequest,
    InvokeMethodResponse,
    InvokeReadMethodRequest,
    InvokeReadMethodResponse,
};
use tonic::{Request, Response, Status};

pub struct ValidatorNodeGrpcServer<TServiceSpecification: ServiceSpecification> {
    node_identity: NodeIdentity,
    db_factory: TServiceSpecification::DbFactory,
    asset_processor: TServiceSpecification::AssetProcessor,
    asset_proxy: TServiceSpecification::AssetProxy,
}

impl<TServiceSpecification: ServiceSpecification> ValidatorNodeGrpcServer<TServiceSpecification> {
    pub fn new(
        node_identity: NodeIdentity,
        db_factory: TServiceSpecification::DbFactory,
        asset_processor: TServiceSpecification::AssetProcessor,
        asset_proxy: TServiceSpecification::AssetProxy,
    ) -> Self {
        Self {
            node_identity,
            db_factory,
            asset_processor,
            asset_proxy,
        }
    }
}

#[tonic::async_trait]
impl<TServiceSpecification: ServiceSpecification + 'static> ValidatorNode
    for ValidatorNodeGrpcServer<TServiceSpecification>
{
    async fn get_identity(
        &self,
        _request: tonic::Request<GetIdentityRequest>,
    ) -> Result<tonic::Response<GetIdentityResponse>, tonic::Status> {
        let response = GetIdentityResponse {
            public_key: self.node_identity.public_key().to_vec(),
            public_address: self.node_identity.public_address().to_string(),
            node_id: self.node_identity.node_id().to_vec(),
        };
        Ok(Response::new(response))
    }

    async fn invoke_method(
        &self,
        request: Request<InvokeMethodRequest>,
    ) -> Result<Response<InvokeMethodResponse>, Status> {
        let request = request.into_inner();
        let contract_id = request
            .contract_id
            .try_into()
            .map_err(|_err| Status::invalid_argument("contract_id was not valid"))?;

        match self
            .asset_proxy
            .invoke_method(
                &contract_id,
                request
                    .template_id
                    .try_into()
                    .map_err(|_| Status::invalid_argument("invalid template_id"))?,
                request.method.clone(),
                request.args.clone(),
                PublicKey::from_bytes(&request.sender).map_err(|_| Status::invalid_argument("invalid sender"))?,
            )
            .await
        {
            Ok(_) => Ok(Response::new(InvokeMethodResponse {
                status: "Accepted".to_string(),
                result: vec![],
            })),
            Err(_) => Ok(Response::new(InvokeMethodResponse {
                status: "Errored".to_string(),
                result: vec![],
            })),
        }
    }

    async fn invoke_read_method(
        &self,
        request: Request<InvokeReadMethodRequest>,
    ) -> Result<Response<InvokeReadMethodResponse>, Status> {
        println!("invoke_read_method grpc call");
        println!("{:?}", request);
        let request = request.into_inner();
        let contract_id = request
            .contract_id
            .try_into()
            .map_err(|err| Status::invalid_argument(format!("Contract ID was not valid: {}", err)))?;
        let template_id = request
            .template_id
            .try_into()
            .map_err(|_| Status::invalid_argument("Invalid template_id"))?;
        if let Some(state) = self
            .db_factory
            .get_state_db(&contract_id)
            .map_err(|e| Status::internal(format!("Could not create state db: {}", e)))?
        {
            let state_db_reader = state.reader();
            let instruction = Instruction::new(
                template_id,
                request.method,
                request.args,
                PublicKey::from_bytes(&request.sender).map_err(|_| Status::invalid_argument("invalid sender"))?,
            );
            let response_bytes = self
                .asset_processor
                .invoke_read_method(&instruction, &state_db_reader)
                .map_err(|e| Status::internal(format!("Could not invoke read method: {}", e)))?;
            Ok(Response::new(InvokeReadMethodResponse {
                result: response_bytes.unwrap_or_default(),
                authority: Some(Authority {
                    node_public_key: vec![],
                    signature: vec![],
                    proxied_by: vec![],
                }),
            }))
        } else {
            // Forward to proxy
            let response_bytes = self
                .asset_proxy
                .invoke_read_method(
                    &contract_id,
                    template_id,
                    request.method,
                    request.args,
                    PublicKey::from_bytes(&request.sender).map_err(|_| Status::invalid_argument("invalid sender"))?,
                )
                .await
                .map_err(|err| Status::internal(format!("Error calling proxied method:{}", err)))?;
            // TODO: Populate authority
            Ok(Response::new(InvokeReadMethodResponse {
                result: response_bytes.unwrap_or_default(),
                authority: Some(Authority {
                    node_public_key: vec![],
                    signature: vec![],
                    proxied_by: vec![],
                }),
            }))
        }
    }
}
