//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub struct Notify<T> {
    publisher: broadcast::Sender<T>,
}

impl<T: Clone> Notify<T> {
    pub fn new(capacity: usize) -> Self {
        let (publisher, _) = broadcast::channel(capacity);
        Self { publisher }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<T> {
        self.publisher.subscribe()
    }

    pub fn notify<V: Into<T>>(&self, value: V) {
        let _err = self.publisher.send(value.into());
    }
}
