use std::str::FromStr;

use tari_app_grpc::{
    authentication::ClientAuthenticationInterceptor,
    tari_rpc::{pow_algo::PowAlgos, wallet_client::WalletClient, NewBlockTemplate, NewBlockTemplateRequest, PowAlgo},
};
use tari_base_node_grpc_client::BaseNodeGrpcClient;
use tari_miner::utils::{coinbase_request, extract_outputs_and_kernels};
use tari_wallet_grpc_client::GrpcAuthentication;
use tonic::{
    codegen::InterceptedService,
    transport::{Channel, Endpoint},
};

use crate::TariWorld;

type BaseNodeClient = BaseNodeGrpcClient<tonic::transport::Channel>;
type WalletGrpcClient = WalletClient<InterceptedService<Channel, ClientAuthenticationInterceptor>>;

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
    let miner = world.miners.get(&miner_name).unwrap();
    let base_node_grpc_port = world.base_nodes.get(&miner.base_node_name).unwrap().grpc_port;
    let wallet_grpc_port = world.wallets.get(&miner.wallet_name).unwrap().grpc_port;

    let base_node_grpc_url = format!("http://127.0.0.1:{}", base_node_grpc_port);
    let mut base_client = BaseNodeClient::connect(base_node_grpc_url).await.unwrap();

    let mut wallet_client = connect_wallet(wallet_grpc_port).await;

    for _ in 0..num_blocks {
        mine_block(&mut base_client, &mut wallet_client).await;
    }
}

async fn mine_block(base_client: &mut BaseNodeClient, wallet_client: &mut WalletGrpcClient) {
    // get block template request
    let template_req = NewBlockTemplateRequest {
        algo: Some(PowAlgo {
            pow_algo: PowAlgos::Sha3.into(),
        }),
        max_weight: 0,
    };
    let template_res = base_client
        .get_new_block_template(template_req)
        .await
        .unwrap()
        .into_inner();
    let mut block_template: NewBlockTemplate = template_res.new_block_template.clone().unwrap();

    // get the coinbase from the wallet
    let coinbase_req = coinbase_request(&template_res).unwrap();
    let coinbase_res = wallet_client.get_coinbase(coinbase_req).await.unwrap().into_inner();

    // add the coinbase to the block
    let (output, kernel) = extract_outputs_and_kernels(coinbase_res).unwrap();
    let body = block_template.body.as_mut().unwrap();
    body.outputs.push(output);
    body.kernels.push(kernel);

    let block_result = base_client
        .get_new_block(block_template.clone())
        .await
        .unwrap()
        .into_inner();
    let block = block_result.block.unwrap();

    let _sumbmit_res = base_client.submit_block(block).await.unwrap();

    println!(
        "Block successfully mined at height {:?}",
        block_template.header.unwrap().height
    );
}

async fn connect_wallet(wallet_grpc_port: u64) -> WalletGrpcClient {
    let wallet_addr = format!("http://127.0.0.1:{}", wallet_grpc_port);
    let channel = Endpoint::from_str(&wallet_addr).unwrap().connect().await.unwrap();
    WalletClient::with_interceptor(
        channel,
        ClientAuthenticationInterceptor::create(&GrpcAuthentication::default()).unwrap(),
    )
}
