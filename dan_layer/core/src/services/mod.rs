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

mod base_node_client;
mod events_publisher;
pub mod infrastructure_services;
pub mod mempool;
mod payload_processor;
mod peer_service;
mod signing_service;
mod wallet_client;

pub use asset_proxy::{AssetProxy, ConcreteAssetProxy};
pub use base_node_client::{BaseNodeClient, BlockInfo, SideChainUtxos};
pub use events_publisher::{EventsPublisher, LoggingEventsPublisher};
pub use payload_processor::{PayloadProcessor, PayloadProcessorError};
pub use peer_service::{DanPeer, PeerProvider};
pub use signing_service::{NodeIdentitySigningService, SigningService};
pub use template_provider::TemplateProvider;
mod asset_proxy;
pub mod base_node_error;
pub mod epoch_manager;
pub mod leader_strategy;
mod service_specification;
mod template_provider;
mod validator_node_rpc_client;

pub use service_specification::ServiceSpecification;
pub use validator_node_rpc_client::{ValidatorNodeClientError, ValidatorNodeClientFactory, ValidatorNodeRpcClient};
pub use wallet_client::WalletClient;
