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

use minotari_app_grpc::{
    tari_rpc,
    tari_rpc::{pow_algo::PowAlgos, GetIdentityRequest, NewBlockTemplate, NewBlockTemplateRequest, PowAlgo},
};
use minotari_node_grpc_client::BaseNodeGrpcClient;
use tari_common::configuration::Network;
use tari_common_types::{tari_address::TariAddress, types::PublicKey};
use tari_core::{
    consensus::ConsensusManager,
    transactions::{
        generate_coinbase_with_wallet_output,
        key_manager::{MemoryDbKeyManager, TariKeyId},
        tari_amount::MicroMinotari,
        transaction_components::{RangeProofType, WalletOutput},
    },
};
use tari_crypto::tari_utilities::ByteArray;

use crate::TariWorld;

type BaseNodeClient = BaseNodeGrpcClient<tonic::transport::Channel>;

#[derive(Debug)]
pub struct MinerProcess {
    pub name: String,
    pub base_node_name: String,
    pub wallet_name: String,
}

pub fn register_miner_process(world: &mut TariWorld, miner_name: String, base_node_name: String, wallet_name: String) {
    let miner = MinerProcess {
        name: miner_name.clone(),
        base_node_name,
        wallet_name,
    };
    world.miners.insert(miner_name, miner);
}

pub async fn mine_blocks(world: &mut TariWorld, miner_name: String, num_blocks: u64) {
    let miner = world.get_miner(&miner_name);
    let mut base_client = create_base_node_client(world, &miner_name).await;
    let mut wallet_client = world.get_wallet(&miner.wallet_name).create_client().await;

    let wallet_pk = PublicKey::from_canonical_bytes(
        &wallet_client
            .identify(GetIdentityRequest {})
            .await
            .unwrap()
            .into_inner()
            .public_key,
    )
    .unwrap();
    let payment_address = TariAddress::new(wallet_pk, Network::LocalNet);

    for _ in 0..num_blocks {
        mine_block(world, &payment_address, &mut base_client).await;
    }
}

async fn create_base_node_client(world: &TariWorld, miner_name: &String) -> BaseNodeClient {
    let miner = world.miners.get(miner_name).unwrap();
    let base_node_grpc_port = world.base_nodes.get(&miner.base_node_name).unwrap().grpc_port;
    let base_node_grpc_url = format!("http://127.0.0.1:{}", base_node_grpc_port);
    eprintln!("Base node GRPC at {}", base_node_grpc_url);
    BaseNodeClient::connect(base_node_grpc_url).await.unwrap()
}

async fn mine_block(world: &TariWorld, payment_address: &TariAddress, base_client: &mut BaseNodeClient) {
    let (block_template, _) = create_block_template_with_coinbase(
        base_client,
        0,
        &world.key_manager,
        &world.script_key_id().await,
        payment_address,
        false,
        &world.consensus_manager,
    )
    .await;

    // Ask the base node for a valid block using the template
    let block_result = base_client
        .get_new_block(block_template.clone())
        .await
        .unwrap()
        .into_inner();
    let block = block_result.block.unwrap();

    // We don't need to mine, as Localnet blocks have difficulty target of 1s
    let submit_res = base_client.submit_block(block).await.unwrap().into_inner();
    log::info!(
        "Block {} successfully mined at height {:?}",
        tari_crypto::tari_utilities::hex::to_hex(&submit_res.block_hash),
        block_template.header.unwrap().height
    );
}
async fn create_block_template_with_coinbase(
    base_client: &mut BaseNodeClient,
    weight: u64,
    key_manager: &MemoryDbKeyManager,
    script_key_id: &TariKeyId,
    wallet_payment_address: &TariAddress,
    stealth_payment: bool,
    consensus_manager: &ConsensusManager,
) -> (NewBlockTemplate, WalletOutput) {
    // get the block template from the base node
    let template_req = NewBlockTemplateRequest {
        algo: Some(PowAlgo {
            pow_algo: PowAlgos::Sha3x.into(),
        }),
        max_weight: weight,
    };

    let template_response = base_client
        .get_new_block_template(template_req)
        .await
        .unwrap()
        .into_inner();

    let mut block_template = template_response.new_block_template.clone().unwrap();

    let template = template_response.new_block_template.as_ref().unwrap();
    let miner_data = template_response.miner_data.as_ref().unwrap();
    let fee = miner_data.total_fees;
    let reward = miner_data.reward;
    let height = template.header.as_ref().unwrap().height;

    // add the coinbase outputs and kernels to the block template
    let (_, coinbase_output, coinbase_kernel, coinbase_wallet_output) = generate_coinbase_with_wallet_output(
        MicroMinotari::from(fee),
        MicroMinotari::from(reward),
        height,
        &[],
        key_manager,
        script_key_id,
        wallet_payment_address,
        stealth_payment,
        consensus_manager.consensus_constants(height),
        RangeProofType::BulletProofPlus,
    )
    .await
    .unwrap();
    let body = block_template.body.as_mut().unwrap();

    let grpc_output = tari_rpc::TransactionOutput::try_from(coinbase_output).unwrap();
    body.outputs.push(grpc_output);
    body.kernels.push(coinbase_kernel.into());

    (block_template, coinbase_wallet_output)
}
