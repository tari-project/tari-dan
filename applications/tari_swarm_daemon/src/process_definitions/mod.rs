//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod context;
mod definition;
mod indexer;
mod minotari_miner;
mod minotari_node;
mod minotari_wallet;
mod signaling_server;
mod validator_node;
mod wallet_daemon;

pub use context::*;
pub use definition::*;

use crate::config::InstanceType;

pub fn get_definition(instance_type: InstanceType) -> Box<dyn ProcessDefinition + 'static> {
    match instance_type {
        InstanceType::MinoTariNode => Box::new(minotari_node::MinotariNode::new()),
        InstanceType::MinoTariConsoleWallet => Box::new(minotari_wallet::MinotariWallet::new()),
        InstanceType::MinoTariMiner => Box::new(minotari_miner::MinotariMiner::new()),
        InstanceType::TariValidatorNode => Box::new(validator_node::ValidatorNode::new()),
        InstanceType::TariWalletDaemon => Box::new(wallet_daemon::WalletDaemon::new()),
        InstanceType::TariIndexer => Box::new(indexer::Indexer::new()),
        InstanceType::TariSignalingServer => Box::new(signaling_server::SignalingServer::new()),
    }
}
