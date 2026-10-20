//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::future::Future;

use minotari_app_grpc::tari_rpc::ValidatorNodeChange;
use tari_node_components::blocks::BlockHeader;
use tari_ootle_common_types::Epoch;
use tari_sidechain::FixedHash;
use tari_template_lib::types::crypto::RistrettoPublicKeyBytes;
use tonic::codegen::tokio_stream::Stream;

use crate::{
    error::BaseNodeClientError,
    types::{BaseLayerConsensusConstants, BaseLayerMetadata, BaseLayerValidatorNode, SideChainUtxos},
};

pub trait BaseNodeClient: Send + Sync + Clone {
    fn test_connection(&mut self) -> impl Future<Output = Result<(), BaseNodeClientError>> + Send;
    fn get_network(&mut self) -> impl Future<Output = Result<u8, BaseNodeClientError>> + Send;
    fn get_tip_info(&mut self) -> impl Future<Output = Result<BaseLayerMetadata, BaseNodeClientError>> + Send;
    fn get_validator_node_changes(
        &mut self,
        epoch: Epoch,
        sidechain_id: Option<&RistrettoPublicKeyBytes>,
    ) -> impl Future<Output = Result<Vec<ValidatorNodeChange>, BaseNodeClientError>> + Send;
    fn get_validator_nodes(
        &mut self,
        height: u64,
    ) -> impl Future<
        Output = Result<
            impl Stream<Item = Result<BaseLayerValidatorNode, BaseNodeClientError>> + Send,
            BaseNodeClientError,
        >,
    > + Send;
    fn get_header_by_hash(
        &mut self,
        block_hash: &FixedHash,
    ) -> impl Future<Output = Result<BlockHeader, BaseNodeClientError>> + Send;
    fn stream_headers(
        &mut self,
        from_height: u64,
        limit: u64,
    ) -> impl Future<
        Output = Result<
            impl Stream<Item = Result<BlockHeader, BaseNodeClientError>> + Unpin + Send,
            BaseNodeClientError,
        >,
    > + Send;
    fn get_consensus_constants(
        &mut self,
        tip: u64,
    ) -> impl Future<Output = Result<BaseLayerConsensusConstants, BaseNodeClientError>> + Send;
    fn get_sidechain_utxos(
        &mut self,
        start_hash: Option<FixedHash>,
        count: u64,
    ) -> impl Future<Output = Result<Vec<SideChainUtxos>, BaseNodeClientError>> + Send;
}
