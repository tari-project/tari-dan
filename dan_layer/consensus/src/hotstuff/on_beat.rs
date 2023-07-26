//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::Arc;

use tokio::sync::watch;

#[derive(Debug, Clone)]
pub struct OnBeat {
    receiver: watch::Receiver<()>,
    sender: Arc<watch::Sender<()>>,
    force_receiver: watch::Receiver<()>,
    force_sender: Arc<watch::Sender<()>>,
}

impl OnBeat {
    pub fn new() -> Self {
        let (sender, receiver) = watch::channel(());
        let (force_sender, force_receiver) = watch::channel(());
        Self {
            receiver,
            sender: Arc::new(sender),
            force_receiver,
            force_sender: Arc::new(force_sender),
        }
    }

    pub async fn wait(&mut self) {
        self.receiver.changed().await.expect("sender can never be dropped")
    }

    pub async fn wait_force_beat(&mut self) {
        self.force_receiver
            .changed()
            .await
            .expect("sender can never be dropped")
    }

    pub fn beat(&self) {
        self.sender.send(()).expect("receiver can never be dropped")
    }

    pub fn force_beat(&self) {
        self.force_sender.send(()).expect("receiver can never be dropped")
    }
}
