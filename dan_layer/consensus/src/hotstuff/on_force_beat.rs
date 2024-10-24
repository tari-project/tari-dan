//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::Arc;

use tari_dan_common_types::NodeHeight;
use tokio::sync::watch;

#[derive(Debug, Clone)]
pub struct OnForceBeat {
    receiver: watch::Receiver<Option<NodeHeight>>,
    sender: Arc<watch::Sender<Option<NodeHeight>>>,
}

impl OnForceBeat {
    pub fn new() -> Self {
        let (sender, receiver) = watch::channel(None);
        Self {
            receiver,
            sender: Arc::new(sender),
        }
    }

    pub async fn wait(&mut self) -> Option<NodeHeight> {
        self.receiver.changed().await.expect("sender can never be dropped");
        *self.receiver.borrow()
    }

    pub fn beat(&self, new_height: Option<NodeHeight>) {
        self.sender.send(new_height).expect("receiver can never be dropped")
    }
}

impl Default for OnForceBeat {
    fn default() -> Self {
        Self::new()
    }
}
