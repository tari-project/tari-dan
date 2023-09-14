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

use std::time::Duration;

use minotari_app_grpc::{
    authentication::ClientAuthenticationInterceptor,
    tari_rpc::{
        pow_algo::PowAlgos,
        wallet_client::WalletClient,
        GetCoinbaseRequest,
        GetCoinbaseResponse,
        NewBlockTemplate,
        NewBlockTemplateRequest,
        NewBlockTemplateResponse,
        PowAlgo,
        TransactionKernel,
        TransactionOutput,
    },
};
use minotari_node_grpc_client::BaseNodeGrpcClient;
use tonic::codegen::InterceptedService;

use crate::TariWorld;

type BaseNodeClient = BaseNodeGrpcClient<tonic::transport::Channel>;
type WalletGrpcClient = WalletClient<InterceptedService<tonic::transport::Channel, ClientAuthenticationInterceptor>>;

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

    for _ in 0..num_blocks {
        mine_block(&mut base_client, &mut wallet_client).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Give some time for the base node and wallet to sync the new blocks
    tokio::time::sleep(Duration::from_secs(5)).await;
}

async fn create_base_node_client(world: &TariWorld, miner_name: &String) -> BaseNodeClient {
    let miner = world.miners.get(miner_name).unwrap();
    let base_node_grpc_port = world.base_nodes.get(&miner.base_node_name).unwrap().grpc_port;
    let base_node_grpc_url = format!("http://127.0.0.1:{}", base_node_grpc_port);
    eprintln!("Base node GRPC at {}", base_node_grpc_url);
    BaseNodeClient::connect(base_node_grpc_url).await.unwrap()
}

async fn mine_block(base_client: &mut BaseNodeClient, wallet_client: &mut WalletGrpcClient) {
    let block_template = create_block_template_with_coinbase(base_client, wallet_client).await;

    // Ask the base node for a valid block using the template
    let block_result = base_client
        .get_new_block(block_template.clone())
        .await
        .unwrap()
        .into_inner();
    let block = block_result.block.unwrap();

    // We don't need to mine, as Localnet blocks have difficulty 1s
    let submit_res = base_client.submit_block(block).await.unwrap().into_inner();
    log::info!(
        "Block {} successfully mined at height {:?}",
        tari_crypto::tari_utilities::hex::to_hex(&submit_res.block_hash),
        block_template.header.unwrap().height
    );
}

async fn create_block_template_with_coinbase(
    base_client: &mut BaseNodeClient,
    wallet_client: &mut WalletGrpcClient,
) -> NewBlockTemplate {
    // get the block template from the base node
    let template_req = NewBlockTemplateRequest {
        algo: Some(PowAlgo {
            pow_algo: PowAlgos::Sha3x.into(),
        }),
        max_weight: 0,
    };
    let template_res = base_client
        .get_new_block_template(template_req)
        .await
        .unwrap()
        .into_inner();

    let mut block_template = template_res.new_block_template.clone().unwrap();

    // add the coinbase outputs and kernels to the block template
    let (output, kernel) = get_coinbase_outputs_and_kernels(wallet_client, template_res).await;
    let body = block_template.body.as_mut().unwrap();
    body.outputs.push(output);
    body.kernels.push(kernel);

    block_template
}

async fn get_coinbase_outputs_and_kernels(
    wallet_client: &mut WalletGrpcClient,
    template_res: NewBlockTemplateResponse,
) -> (TransactionOutput, TransactionKernel) {
    let coinbase_req = coinbase_request(&template_res);
    let coinbase_res = wallet_client.get_coinbase(coinbase_req).await.unwrap().into_inner();
    extract_outputs_and_kernels(coinbase_res)
}

fn coinbase_request(template_response: &NewBlockTemplateResponse) -> GetCoinbaseRequest {
    let template = template_response.new_block_template.as_ref().unwrap();
    let miner_data = template_response.miner_data.as_ref().unwrap();
    let fee = miner_data.total_fees;
    let reward = miner_data.reward;
    let height = template.header.as_ref().unwrap().height;
    let extra = vec![];
    GetCoinbaseRequest {
        reward,
        fee,
        height,
        extra,
    }
}

fn extract_outputs_and_kernels(coinbase: GetCoinbaseResponse) -> (TransactionOutput, TransactionKernel) {
    let transaction_body = coinbase.transaction.unwrap().body.unwrap();
    let output = transaction_body.outputs.get(0).cloned().unwrap();
    let kernel = transaction_body.kernels.get(0).cloned().unwrap();
    (output, kernel)
}
