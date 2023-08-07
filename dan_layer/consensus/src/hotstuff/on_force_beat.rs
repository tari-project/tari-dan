//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::Arc;

use tokio::sync::watch;

#[derive(Debug, Clone)]
pub struct OnForceBeat {
    receiver: watch::Receiver<()>,
    sender: Arc<watch::Sender<()>>,
}

impl OnForceBeat {
    pub fn new() -> Self {
        let (sender, receiver) = watch::channel(());
        Self {
            receiver,
            sender: Arc::new(sender),
        }
    }

    pub async fn wait(&mut self) {
        self.receiver.changed().await.expect("sender can never be dropped")
    }

    pub fn beat(&self) {
        self.sender.send(()).expect("receiver can never be dropped")
    }
}
