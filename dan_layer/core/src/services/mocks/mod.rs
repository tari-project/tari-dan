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

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use tari_common_types::types::{FixedHash, PublicKey};
use tari_comms::types::CommsPublicKey;
use tari_core::{chain_storage::UtxoMinedInfo, transactions::transaction_components::OutputType};
use tari_crypto::ristretto::RistrettoPublicKey;
#[cfg(test)]
use tari_dan_engine::state::mocks::state_db::MockStateDbBackupAdapter;
use tari_dan_engine::{
    instruction::Transaction,
    state::{
        models::{SchemaState, StateOpLogEntry, StateRoot},
        StateDbUnitOfWork,
        StateDbUnitOfWorkReader,
    },
};

use super::mempool::service::MempoolService;
use crate::{
    digital_assets_error::DigitalAssetError,
    models::{
        BaseLayerMetadata,
        BaseLayerOutput,
        Committee,
        Event,
        HotStuffTreeNode,
        Node,
        Payload,
        SidechainMetadata,
        TariDanPayload,
        TreeNodeHash,
        ValidatorSignature,
    },
    services::{
        base_node_client::BaseNodeClient,
        infrastructure_services::NodeAddressable,
        EventsPublisher,
        PayloadProcessor,
        SigningService,
        ValidatorNodeClientError,
        ValidatorNodeClientFactory,
        ValidatorNodeRpcClient,
    },
    storage::{chain::ChainDbUnitOfWork, ChainStorageService, StorageError},
};
#[cfg(test)]
use crate::{
    models::domain_events::ConsensusWorkerDomainEvent,
    services::infrastructure_services::mocks::{MockInboundConnectionService, MockOutboundService},
    services::{ConcreteAssetProxy, ServiceSpecification},
    storage::mocks::{chain_db::MockChainDbBackupAdapter, MockDbFactory},
};

#[derive(Debug, Clone)]
pub struct MockMempoolService;

#[async_trait]
impl MempoolService for MockMempoolService {
    async fn submit_transaction(&mut self, _transaction: &Transaction) -> Result<(), DigitalAssetError> {
        Ok(())
    }

    async fn size(&self) -> usize {
        0
    }
}

pub fn create_mempool_mock() -> MockMempoolService {
    MockMempoolService
}

pub fn mock_events_publisher<TEvent: Event>() -> MockEventsPublisher<TEvent> {
    MockEventsPublisher::default()
}

#[derive(Clone)]
pub struct MockEventsPublisher<TEvent: Event> {
    events: Arc<Mutex<VecDeque<TEvent>>>,
}

impl<TEvent: Event> Default for MockEventsPublisher<TEvent> {
    fn default() -> Self {
        Self {
            events: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
}

impl<TEvent: Event> MockEventsPublisher<TEvent> {
    pub fn to_vec(&self) -> Vec<TEvent> {
        self.events.lock().unwrap().iter().cloned().collect()
    }
}

impl<TEvent: Event> EventsPublisher<TEvent> for MockEventsPublisher<TEvent> {
    fn publish(&mut self, event: TEvent) {
        self.events.lock().unwrap().push_back(event)
    }
}

pub fn mock_signing_service() -> MockSigningService {
    MockSigningService
}

pub struct MockSigningService;

impl SigningService for MockSigningService {
    fn sign(&self, _challenge: &[u8]) -> Result<ValidatorSignature, DigitalAssetError> {
        Ok(ValidatorSignature { signer: vec![8u8; 32] })
    }
}

#[derive(Clone)]
pub struct MockBaseNodeClient {}

#[async_trait]
impl BaseNodeClient for MockBaseNodeClient {
    async fn get_tip_info(&mut self) -> Result<BaseLayerMetadata, DigitalAssetError> {
        todo!();
    }
}

pub fn mock_base_node_client() -> MockBaseNodeClient {
    MockBaseNodeClient {}
}

// pub fn _mock_template_service() -> MockTemplateService {
//     MockTemplateService {}
// }
//
// pub struct MockTemplateService {}
//
// #[async_trait]
// impl TemplateService for MockTemplateService {
//     async fn execute_instruction(&mut self, _instruction: &Instruction) -> Result<(), DigitalAssetError> {
//         dbg!("Executing instruction as mock");
//         Ok(())
//     }
// }

pub fn mock_payload_processor() -> MockPayloadProcessor {
    MockPayloadProcessor {}
}

pub struct MockPayloadProcessor {}

#[async_trait]
impl<TPayload: Payload> PayloadProcessor<TPayload> for MockPayloadProcessor {
    async fn process_payload<TUnitOfWork: StateDbUnitOfWork>(
        &self,
        _payload: &TPayload,
        _unit_of_work: TUnitOfWork,
    ) -> Result<StateRoot, DigitalAssetError> {
        todo!()
    }
}

#[derive(Default, Clone)]
pub struct MockValidatorNodeClientFactory;

#[derive(Default, Clone)]
pub struct MockValidatorNodeClient;

#[async_trait]
impl ValidatorNodeRpcClient for MockValidatorNodeClient {
    async fn submit_transaction(
        &mut self,
        _transaction: Transaction,
    ) -> Result<Option<Vec<u8>>, ValidatorNodeClientError> {
        Ok(None)
    }

    async fn get_sidechain_state(
        &mut self,
        _contract_id: &FixedHash,
    ) -> Result<Vec<SchemaState>, ValidatorNodeClientError> {
        Ok(vec![])
    }

    async fn get_op_logs(
        &mut self,
        _contract_id: &FixedHash,
        _height: u64,
    ) -> Result<Vec<StateOpLogEntry>, ValidatorNodeClientError> {
        Ok(vec![])
    }

    async fn get_tip_node(&mut self, _contract_id: &FixedHash) -> Result<Option<Node>, ValidatorNodeClientError> {
        Ok(None)
    }
}

impl ValidatorNodeClientFactory for MockValidatorNodeClientFactory {
    type Addr = PublicKey;
    type Client = MockValidatorNodeClient;

    fn create_client(&self, _address: &Self::Addr) -> Self::Client {
        MockValidatorNodeClient::default()
    }
}

#[derive(Default, Clone)]
pub struct MockChainStorageService;

#[async_trait]
impl ChainStorageService<CommsPublicKey> for MockChainStorageService {
    async fn get_metadata(&self) -> Result<SidechainMetadata, StorageError> {
        todo!()
    }

    async fn add_node<TUnitOfWork: ChainDbUnitOfWork>(
        &self,
        _node: &HotStuffTreeNode<CommsPublicKey>,
        _db: TUnitOfWork,
    ) -> Result<(), StorageError> {
        Ok(())
    }
}

pub fn create_public_key() -> RistrettoPublicKey {
    let mut rng = rand::thread_rng();
    let (_, address) = <RistrettoPublicKey as tari_crypto::keys::PublicKey>::random_keypair(&mut rng);
    address
}

#[derive(Default, Clone)]
pub struct MockServiceSpecification;

#[cfg(test)]
impl ServiceSpecification for MockServiceSpecification {
    type Addr = RistrettoPublicKey;
    type AssetProxy = ConcreteAssetProxy<Self>;
    type BaseNodeClient = MockBaseNodeClient;
    type ChainDbBackendAdapter = MockChainDbBackupAdapter;
    type CheckpointManager = ConcreteCheckpointManager<Self::WalletClient>;
    type CommitteeManager = MockCommitteeManager;
    type DbFactory = MockDbFactory;
    type EventsPublisher = MockEventsPublisher<ConsensusWorkerDomainEvent>;
    type GlobalDbAdapter = crate::storage::mocks::global_db::MockGlobalDbBackupAdapter;
    type InboundConnectionService = MockInboundConnectionService<Self::Addr, Self::Payload>;
    type MempoolService = MockMempoolService;
    type OutboundService = MockOutboundService<Self::Addr, Self::Payload>;
    type Payload = TariDanPayload;
    type PayloadProcessor = MockPayloadProcessor;
    type PayloadProvider = MockStaticPayloadProvider<Self::Payload>;
    type SigningService = MockSigningService;
    type StateDbBackendAdapter = MockStateDbBackupAdapter;
    type ValidatorNodeClientFactory = MockValidatorNodeClientFactory;
    type WalletClient = MockWalletClient;
}
