//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::sync::Arc;

use tari_dan_common_types::NodeHeight;
use tokio::sync::watch;

#[derive(Debug, Clone)]
pub struct OnLeaderTimeout {
    // todo: consider using a different sync construct, like an mpsc channel
    receiver: watch::Receiver<NodeHeight>,
    sender: Arc<watch::Sender<NodeHeight>>,
}

impl OnLeaderTimeout {
    pub fn new() -> Self {
        let (sender, receiver) = watch::channel(NodeHeight::zero());
        Self {
            receiver,
            sender: Arc::new(sender),
        }
    }

    pub async fn wait(&mut self) -> NodeHeight {
        self.receiver.changed().await.expect("sender can never be dropped");
        // This could lead to a more recent value being seen. Idk if that is ok...
        *self.receiver.borrow()
    }

    pub fn leader_timed_out(&self, new_height: NodeHeight) {
        self.sender.send(new_height).expect("receiver can never be dropped")
    }
}

impl Default for OnLeaderTimeout {
    fn default() -> Self {
        Self::new()
    }
}
