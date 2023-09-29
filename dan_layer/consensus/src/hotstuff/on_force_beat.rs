//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::Arc;

use tari_dan_storage::consensus_models::LeafBlock;
use tokio::sync::watch;

#[derive(Debug, Clone)]
pub struct OnForceBeat {
    receiver: watch::Receiver<Option<LeafBlock>>,
    sender: Arc<watch::Sender<Option<LeafBlock>>>,
}

impl OnForceBeat {
    pub fn new() -> Self {
        let (sender, receiver) = watch::channel(None);
        Self {
            receiver,
            sender: Arc::new(sender),
        }
    }

    pub async fn wait(&mut self) -> Option<LeafBlock> {
        self.receiver.changed().await.expect("sender can never be dropped");
        self.receiver.borrow().clone()
    }

    pub fn beat(&self, parent_block: Option<LeafBlock>) {
        self.sender.send(parent_block).expect("receiver can never be dropped")
    }
}
