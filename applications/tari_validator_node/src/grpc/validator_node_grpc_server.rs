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

use tari_comms::NodeIdentity;
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_core::services::{AssetProxy, ServiceSpecification};
use tari_validator_node_grpc::rpc::{
    validator_node_server::ValidatorNode,
    GetIdentityRequest,
    GetIdentityResponse,
    SubmitTransactionRequest,
    SubmitTransactionResponse,
};
use tonic::{Request, Response, Status};

pub struct ValidatorNodeGrpcServer<TServiceSpecification: ServiceSpecification> {
    node_identity: NodeIdentity,
    _db_factory: TServiceSpecification::DbFactory,
    asset_proxy: TServiceSpecification::AssetProxy,
}

impl<TServiceSpecification: ServiceSpecification> ValidatorNodeGrpcServer<TServiceSpecification> {
    pub fn new(
        node_identity: NodeIdentity,
        _db_factory: TServiceSpecification::DbFactory,
        asset_proxy: TServiceSpecification::AssetProxy,
    ) -> Self {
        Self {
            node_identity,
            _db_factory,
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

    async fn submit_transaction(
        &self,
        request: Request<SubmitTransactionRequest>,
    ) -> Result<Response<SubmitTransactionResponse>, Status> {
        let transaction = request
            .into_inner()
            .try_into()
            .map_err(|err| Status::invalid_argument(format!("Transaction was not valid: {}", err)))?;

        match self.asset_proxy.submit_transaction(&transaction).await {
            Ok(result) => Ok(Response::new(SubmitTransactionResponse {
                status: "Accepted".to_string(),
                result,
            })),
            Err(err) => Ok(Response::new(SubmitTransactionResponse {
                status: format!("Errored: {}", err),
                result: vec![],
            })),
        }
    }
}
