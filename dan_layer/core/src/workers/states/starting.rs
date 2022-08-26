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

use std::{convert::TryInto, marker::PhantomData};

use log::*;
use tari_core::transactions::transaction_components::OutputType;

use crate::{
    digital_assets_error::DigitalAssetError,
    services::{BaseNodeClient, ServiceSpecification},
    storage::DbFactory,
    workers::states::ConsensusWorkerStateEvent,
};

const LOG_TARGET: &str = "tari::dan::workers::states::starting";

#[derive(Default)]
pub struct Starting<TSpecification> {
    _spec: PhantomData<TSpecification>,
}

impl<TSpecification: ServiceSpecification> Starting<TSpecification> {
    pub fn new() -> Self {
        Default::default()
    }

    pub async fn next_event(
        &self,
        base_node_client: &mut TSpecification::BaseNodeClient,
        db_factory: &TSpecification::DbFactory,
        node_id: &TSpecification::Addr,
    ) -> Result<ConsensusWorkerStateEvent, DigitalAssetError>
    where
        TSpecification: ServiceSpecification,
    {
        info!(
            target: LOG_TARGET,
            "Checking base layer to see if we are part of the committee"
        );
        todo!()
        // let tip = base_node_client.get_tip_info().await?;
        // // get latest checkpoint on the base layer
        // let mut outputs = base_node_client
        //     .get_current_contract_outputs(
        //         tip.height_of_longest_chain
        //             .saturating_sub(asset_definition.base_layer_confirmation_time),
        //         asset_definition.contract_id,
        //         OutputType::ContractConstitution,
        //     )
        //     .await?;
        //
        // let output = match outputs.pop() {
        //     Some(chk) => chk.try_into()?,
        //     None => return Ok(ConsensusWorkerStateEvent::BaseLayerCheckopintNotFound),
        // };
        //
        // committee_manager.read_from_constitution(output)?;
        //
        // if !committee_manager.current_committee()?.contains(node_id) {
        //     info!(
        //         target: LOG_TARGET,
        //         "Validator node not part of committee for asset public key '{}'", asset_definition.contract_id
        //     );
        //     return Ok(ConsensusWorkerStateEvent::NotPartOfCommittee);
        // }
        //
        // info!(
        //     target: LOG_TARGET,
        //     "Validator node is a committee member for asset public key '{}'", asset_definition.contract_id
        // );
        // // read and create the genesis block
        // info!(target: LOG_TARGET, "Creating DB");
        // let _chain_db = db_factory.get_or_create_chain_db(&asset_definition.contract_id)?;
        //
        // Ok(ConsensusWorkerStateEvent::Initialized)
    }
}
