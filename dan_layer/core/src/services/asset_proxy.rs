//  Copyright 2021. The Tari Project
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

use async_trait::async_trait;
use tari_common_types::types::PublicKey;
use tari_dan_engine::instruction::Transaction;

use super::mempool::service::MempoolService;
use crate::{services::ServiceSpecification, DigitalAssetError};

#[async_trait]
pub trait AssetProxy: Send + Sync {
    async fn submit_transaction(&self, transaction: &Transaction) -> Result<Vec<u8>, DigitalAssetError>;
}

#[derive(Clone)]
pub struct ConcreteAssetProxy<TServiceSpecification: ServiceSpecification> {
    _base_node_client: TServiceSpecification::BaseNodeClient,
    // _validator_node_client_factory: TServiceSpecification::ValidatorNodeClientFactory,
    _max_clients_to_ask: usize,
    mempool: TServiceSpecification::MempoolService,
    _db_factory: TServiceSpecification::DbFactory,
}

impl<TServiceSpecification: ServiceSpecification<Addr = PublicKey>> ConcreteAssetProxy<TServiceSpecification> {
    pub fn new(
        _base_node_client: TServiceSpecification::BaseNodeClient,
        // _validator_node_client_factory: TServiceSpecification::ValidatorNodeClientFactory,
        _max_clients_to_ask: usize,
        mempool: TServiceSpecification::MempoolService,
        _db_factory: TServiceSpecification::DbFactory,
    ) -> Self {
        Self {
            _base_node_client,
            // _validator_node_client_factory,
            _max_clients_to_ask,
            mempool,
            _db_factory,
        }
    }
}

#[async_trait]
impl<TServiceSpecification: ServiceSpecification<Addr = PublicKey>> AssetProxy
    for ConcreteAssetProxy<TServiceSpecification>
{
    async fn submit_transaction(&self, transaction: &Transaction) -> Result<Vec<u8>, DigitalAssetError> {
        // TODO: validate the transaction signature
        // TODO: check if this VN should process the instruction
        // TODO: process the instruction in the engine
        // TODO: update the state and reach consensus

        let mut mempool = self.mempool.clone();
        mempool.submit_transaction(transaction).await?;

        Ok(vec![])
    }
}
