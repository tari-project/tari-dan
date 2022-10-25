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

use std::{
    collections::HashMap,
    convert::{TryFrom, TryInto},
    net::SocketAddr,
    num::NonZeroU64,
    ops::RangeInclusive,
};

use async_trait::async_trait;
use log::info;
use tari_app_grpc::tari_rpc::{self as grpc, GetCommitteeRequest, GetShardKeyRequest};
use tari_base_node_grpc_client::BaseNodeGrpcClient;
use tari_common_types::types::{FixedHash, PublicKey};
use tari_comms::types::CommsPublicKey;
use tari_core::{
    blocks::BlockHeader,
    consensus::{
        consensus_constants::{OutputVersionRange, PowAlgorithmConstants},
        ConsensusConstants,
    },
    proof_of_work::{Difficulty, PowAlgorithm},
    transactions::{
        tari_amount::MicroTari,
        transaction_components::{
            CodeTemplateRegistration,
            OutputFeaturesVersion,
            OutputType,
            TransactionInputVersion,
            TransactionKernelVersion,
            TransactionOutputVersion,
        },
        weight::{TransactionWeight, WeightParams},
    },
};
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_common_types::ShardId;
use tari_dan_core::{
    models::{BaseLayerMetadata, ValidatorNode},
    services::{base_node_error::BaseNodeError, BaseNodeClient, BlockInfo, SideChainUtxos},
};

const LOG_TARGET: &str = "tari::validator_node::app";

type Client = BaseNodeGrpcClient<tonic::transport::Channel>;

#[derive(Clone)]
pub struct GrpcBaseNodeClient {
    endpoint: SocketAddr,
    client: Option<Client>,
}

impl GrpcBaseNodeClient {
    pub fn new(endpoint: SocketAddr) -> GrpcBaseNodeClient {
        Self { endpoint, client: None }
    }

    async fn connection(&mut self) -> Result<&mut Client, BaseNodeError> {
        if self.client.is_none() {
            let url = format!("http://{}", self.endpoint);
            let inner = Client::connect(url).await?;
            self.client = Some(inner);
        }
        self.client.as_mut().ok_or(BaseNodeError::ConnectionError)
    }
}
#[async_trait]
impl BaseNodeClient for GrpcBaseNodeClient {
    async fn get_tip_info(&mut self) -> Result<BaseLayerMetadata, BaseNodeError> {
        let inner = self.connection().await?;
        let request = grpc::Empty {};
        let result = inner.get_tip_info(request).await?.into_inner();
        let metadata = result
            .metadata
            .ok_or_else(|| BaseNodeError::InvalidPeerMessage("Base node returned no metadata".to_string()))?;
        Ok(BaseLayerMetadata {
            height_of_longest_chain: metadata.height_of_longest_chain,
            tip_hash: metadata
                .best_block
                .try_into()
                .map_err(|_| BaseNodeError::InvalidPeerMessage("best_block was not a valid fixed hash".to_string()))?,
        })
    }

    async fn get_validator_nodes(&mut self, height: u64) -> Result<Vec<ValidatorNode>, BaseNodeError> {
        let inner = self.connection().await?;
        let request = grpc::GetActiveValidatorNodesRequest { height };
        dbg!(&request);
        let mut vns = vec![];
        let mut stream = inner.get_active_validator_nodes(request).await?.into_inner();
        loop {
            match stream.message().await {
                Ok(Some(val)) => {
                    vns.push(ValidatorNode {
                        public_key: CommsPublicKey::from_bytes(&val.public_key).map_err(|_| {
                            BaseNodeError::InvalidPeerMessage("public_key was not a valid public key".to_string())
                        })?,
                        shard_key: ShardId::from_bytes(&val.shard_key).map_err(|_| {
                            BaseNodeError::InvalidPeerMessage("shard_id was not a valid fixed hash".to_string())
                        })?,
                    });
                },
                Ok(None) => {
                    info!(target: LOG_TARGET, "No new validator nodes for this epoch");
                    break;
                },
                Err(e) => {
                    return Err(BaseNodeError::InvalidPeerMessage(format!(
                        "Error reading stream: {}",
                        e
                    )));
                },
            }
        }
        Ok(vns)
    }

    async fn get_committee(&mut self, height: u64, shard_key: &[u8; 32]) -> Result<Vec<CommsPublicKey>, BaseNodeError> {
        let inner = self.connection().await?;
        let request = GetCommitteeRequest {
            height,
            shard_key: shard_key.to_vec(),
        };
        let result = inner.get_committee(request).await?.into_inner();
        Ok(result
            .public_key
            .iter()
            .map(|a| CommsPublicKey::from_vec(a).unwrap())
            .collect())
    }

    async fn get_shard_key(&mut self, height: u64, public_key: &PublicKey) -> Result<Option<ShardId>, BaseNodeError> {
        let inner = self.connection().await?;
        let request = GetShardKeyRequest {
            height,
            public_key: public_key.to_vec(),
        };
        let result = inner.get_shard_key(request).await?.into_inner();
        if result.shard_key.is_empty() {
            Ok(None)
        } else {
            Ok(Some(ShardId::from_bytes(result.shard_key.as_bytes())?))
        }
    }

    async fn get_consensus_constants(&mut self) -> Result<ConsensusConstants, BaseNodeError> {
        let inner = self.connection().await?;
        let request = grpc::Empty {};
        let result = inner.get_constants(request).await?.into_inner();

        let blockchain_version_range =
            result
                .valid_blockchain_version_range
                .ok_or(BaseNodeError::InvalidPeerMessage(
                    "Unavailable data for requested blockchain version".to_string(),
                ))?;

        let lower_blockchain_version = u16::try_from(blockchain_version_range.min)
            .map_err(|e| BaseNodeError::InvalidPeerMessage(e.to_string()))?;

        let upper_blockchain_version = u16::try_from(blockchain_version_range.max)
            .map_err(|e| BaseNodeError::InvalidPeerMessage(e.to_string()))?;

        let valid_blockchain_version_range = RangeInclusive::new(lower_blockchain_version, upper_blockchain_version);

        let mut proof_of_work = HashMap::<PowAlgorithm, PowAlgorithmConstants>::new();

        if let Some(pow) = result.proof_of_work.get(&0u32) {
            let pow = PowAlgorithmConstants {
                max_target_time: pow.max_target_time,
                min_difficulty: Difficulty::from_u64(pow.min_difficulty),
                max_difficulty: Difficulty::from_u64(pow.max_difficulty),
                target_time: pow.target_time,
            };
            proof_of_work.insert(PowAlgorithm::Monero, pow);
        }

        if let Some(pow) = result.proof_of_work.get(&1u32) {
            let pow = PowAlgorithmConstants {
                max_target_time: pow.max_target_time,
                min_difficulty: Difficulty::from_u64(pow.min_difficulty),
                max_difficulty: Difficulty::from_u64(pow.max_difficulty),
                target_time: pow.target_time,
            };
            proof_of_work.insert(PowAlgorithm::Sha3, pow);
        }

        let requested_transaction_weight = result.transaction_weight.ok_or(BaseNodeError::InvalidPeerMessage(
            "Unavailable data for requested transaction weight".to_string(),
        ))?;

        let kernel_weight = requested_transaction_weight.kernel_weight;
        let input_weight = requested_transaction_weight.input_weight;
        let output_weight = requested_transaction_weight.output_weight;
        let metadata_bytes_per_gram = NonZeroU64::new(requested_transaction_weight.metadata_bytes_per_gram);

        let transaction_weight = TransactionWeight::new(WeightParams {
            kernel_weight,
            input_weight,
            output_weight,
            metadata_bytes_per_gram,
        });

        let input_version_range = result.input_version_range.ok_or(BaseNodeError::InvalidPeerMessage(
            "Unavailable data for requested input version".to_string(),
        ))?;
        let lower_input_version = match input_version_range.min {
            0u64 => TransactionInputVersion::V0,
            1 => TransactionInputVersion::V1,
            _ => {
                return Err(BaseNodeError::InvalidPeerMessage(
                    "Failed to parse lower input version".to_string(),
                ))
            },
        };
        let upper_input_version = match input_version_range.max {
            0u64 => TransactionInputVersion::V0,
            1 => TransactionInputVersion::V1,
            _ => {
                return Err(BaseNodeError::InvalidPeerMessage(
                    "Failed to parse upper input version".to_string(),
                ))
            },
        };

        let input_version_range = RangeInclusive::new(lower_input_version, upper_input_version);

        let requested_output_version_range = result.output_version_range.ok_or(BaseNodeError::InvalidPeerMessage(
            "Unavailable data for requested output version range".to_string(),
        ))?;

        let outputs = requested_output_version_range
            .outputs
            .ok_or(BaseNodeError::InvalidPeerMessage(
                "Unavailable data for requested output version range outputs".to_string(),
            ))?;
        let features = requested_output_version_range
            .features
            .ok_or(BaseNodeError::InvalidPeerMessage(
                "Unavailable data for requested output version range features".to_string(),
            ))?;

        let lower_output_version = match outputs.min {
            0u64 => TransactionOutputVersion::V0,
            1 => TransactionOutputVersion::V1,
            _ => {
                return Err(BaseNodeError::InvalidPeerMessage(
                    "Failed to parse lower output version".to_string(),
                ))
            },
        };
        let upper_output_version = match outputs.max {
            0u64 => TransactionOutputVersion::V0,
            1 => TransactionOutputVersion::V1,
            _ => {
                return Err(BaseNodeError::InvalidPeerMessage(
                    "Failed to parse upper output version".to_string(),
                ))
            },
        };

        let outputs = RangeInclusive::new(lower_output_version, upper_output_version);

        let lower_features_version = match features.min {
            0u64 => OutputFeaturesVersion::V0,
            1 => OutputFeaturesVersion::V1,
            _ => {
                return Err(BaseNodeError::InvalidPeerMessage(
                    "Failed to parse lower features version".to_string(),
                ))
            },
        };
        let upper_features_version = match features.max {
            0u64 => OutputFeaturesVersion::V0,
            1 => OutputFeaturesVersion::V1,
            _ => {
                return Err(BaseNodeError::InvalidPeerMessage(
                    "Failed to parse upper features version".to_string(),
                ))
            },
        };

        let features = RangeInclusive::new(lower_features_version, upper_features_version);

        let output_version_range = OutputVersionRange { outputs, features };

        let requested_kernel_version_range = result.kernel_version_range.ok_or(BaseNodeError::InvalidPeerMessage(
            "Unavailable data for requested kernel version range".to_string(),
        ))?;

        let lower_kernel_version = match requested_kernel_version_range.min {
            0u64 => TransactionKernelVersion::V0,
            _ => {
                return Err(BaseNodeError::InvalidPeerMessage(
                    "Failed to parse transaction kernel version correctly".to_string(),
                ))
            },
        };
        let upper_kernel_version = match requested_kernel_version_range.max {
            0u64 => TransactionKernelVersion::V0,
            _ => {
                return Err(BaseNodeError::InvalidPeerMessage(
                    "Failed to parse transaction kernel version correctly".to_string(),
                ))
            },
        };

        let kernel_version_range = RangeInclusive::new(lower_kernel_version, upper_kernel_version);

        let permitted_output_types = result.permitted_output_types;
        let permitted_output_types = permitted_output_types
            .iter()
            .map(|&ut| match ut {
                0i32 => Ok(OutputType::Standard),
                1 => Ok(OutputType::Coinbase),
                2 => Ok(OutputType::Burn),
                3 => Ok(OutputType::ValidatorNodeRegistration),
                4 => Ok(OutputType::CodeTemplateRegistration),
                _ => {
                    return Err(BaseNodeError::InvalidPeerMessage(
                        "Failed to parse permitted output types".to_string(),
                    ))
                },
            })
            .collect::<Result<Vec<_>, _>>()?;

        let validator_node_timeout = result.validator_node_timeout;

        let consensus_constants = ConsensusConstants {
            effective_from_height: result.effective_from_height,
            coinbase_lock_height: result.coinbase_lock_height,
            blockchain_version: u16::try_from(result.blockchain_version)
                .map_err(|e| BaseNodeError::InvalidPeerMessage(e.to_string()))?,
            valid_blockchain_version_range,
            future_time_limit: result.future_time_limit,
            difficulty_block_window: result.difficulty_block_window,
            max_block_transaction_weight: result.max_block_transaction_weight,
            median_timestamp_count: usize::try_from(result.median_timestamp_count)
                .map_err(|e| BaseNodeError::InvalidPeerMessage(e.to_string()))?,
            emission_initial: MicroTari(result.emission_initial),
            emission_decay: result.emission_decay.leak(),
            emission_tail: MicroTari(result.emission_tail),
            max_randomx_seed_height: result.max_randomx_seed_height,
            proof_of_work,
            faucet_value: MicroTari(result.faucet_value),
            transaction_weight,
            max_script_byte_size: result.max_script_byte_size as usize,
            input_version_range,
            output_version_range,
            kernel_version_range,
            permitted_output_types: permitted_output_types.leak(),
            validator_node_timeout,
        };
        Ok(consensus_constants)
    }

    async fn get_template_registrations(
        &mut self,
        start_hash: Option<FixedHash>,
        count: u64,
    ) -> Result<Vec<CodeTemplateRegistration>, BaseNodeError> {
        let inner = self.connection().await?;
        let request = grpc::GetTemplateRegistrationsRequest {
            start_hash: start_hash.map(|v| v.to_vec()).unwrap_or_default(),
            count,
        };
        dbg!(&request);
        let mut templates = vec![];
        let mut stream = inner.get_template_registrations(request).await?.into_inner();
        loop {
            match stream.message().await {
                Ok(Some(val)) => {
                    let template_registration: CodeTemplateRegistration = val
                        .registration
                        .ok_or_else(|| {
                            BaseNodeError::InvalidPeerMessage("Base node returned no template registration".to_string())
                        })?
                        .try_into()
                        .map_err(|_| BaseNodeError::InvalidPeerMessage("invalid template registration".to_string()))?;
                    templates.push(template_registration);
                },
                Ok(None) => {
                    break;
                },
                Err(e) => {
                    return Err(BaseNodeError::InvalidPeerMessage(format!(
                        "Error reading stream: {}",
                        e
                    )));
                },
            }
        }
        Ok(templates)
    }

    async fn get_header_by_hash(&mut self, block_hash: FixedHash) -> Result<BlockHeader, BaseNodeError> {
        let inner = self.connection().await?;
        let request = grpc::GetHeaderByHashRequest {
            hash: block_hash.to_vec(),
        };
        let result = inner.get_header_by_hash(request).await?.into_inner();
        let header = result
            .header
            .ok_or_else(|| BaseNodeError::InvalidPeerMessage("Base node returned no header".to_string()))?;
        let header = header.try_into().map_err(BaseNodeError::InvalidPeerMessage)?;
        Ok(header)
    }

    async fn get_sidechain_utxos(
        &mut self,
        start_hash: Option<FixedHash>,
        count: u64,
    ) -> Result<Vec<SideChainUtxos>, BaseNodeError> {
        let inner = self.connection().await?;
        let request = grpc::GetSideChainUtxosRequest {
            start_hash: start_hash.map(|v| v.to_vec()).unwrap_or_default(),
            count,
        };
        let mut stream = inner.get_side_chain_utxos(request).await?.into_inner();
        let mut responses = Vec::with_capacity(count as usize);
        loop {
            match stream.message().await {
                Ok(Some(resp)) => {
                    let block_info = resp.block_info.ok_or_else(|| {
                        BaseNodeError::InvalidPeerMessage("Base node returned no block info".to_string())
                    })?;
                    let resp = SideChainUtxos {
                        block_info: BlockInfo {
                            height: block_info.height,
                            hash: block_info.hash.try_into()?,
                            next_block_hash: Some(block_info.next_block_hash)
                                .filter(|v| !v.is_empty())
                                .map(TryInto::try_into)
                                .transpose()?,
                        },
                        outputs: resp
                            .outputs
                            .into_iter()
                            .map(TryInto::try_into)
                            .collect::<Result<_, _>>()
                            .map_err(BaseNodeError::InvalidPeerMessage)?,
                    };
                    responses.push(resp);
                },
                Ok(None) => {
                    break;
                },
                Err(e) => {
                    return Err(BaseNodeError::InvalidPeerMessage(format!(
                        "Error reading stream: {}",
                        e
                    )));
                },
            }
        }

        Ok(responses)
    }
}
