//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use prometheus::{core::Collector, Registry};

pub trait CollectorRegister {
    fn register_at(self, registry: &Registry) -> Self;
}

impl<C: Collector + Clone + 'static> CollectorRegister for C {
    fn register_at(self, registry: &Registry) -> Self {
        registry.register(Box::new(self.clone())).unwrap();
        self
    }
}
