//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use prometheus::{
    core::{Collector, MetricVec, MetricVecBuilder},
    Registry,
};

pub trait CollectorRegister {
    fn register_at(self, registry: &Registry) -> Self;
}

impl<C: Collector + Clone + 'static> CollectorRegister for C {
    fn register_at(self, registry: &Registry) -> Self {
        registry.register(Box::new(self.clone())).unwrap();
        self
    }
}

pub trait LabelledCollector<T: MetricVecBuilder> {
    #[allow(dead_code)]
    fn with_label<L: ToString + ?Sized>(&self, label: &L) -> T::M;
    fn with_two_labels<L1: ToString + ?Sized, L2: ToString + ?Sized>(&self, label1: &L1, label2: &L2) -> T::M;
}

impl<T: MetricVecBuilder> LabelledCollector<T> for MetricVec<T> {
    fn with_label<L: ToString + ?Sized>(&self, label: &L) -> T::M {
        self.with_label_values(&[label.to_string().as_str()])
    }

    fn with_two_labels<L1: ToString + ?Sized, L2: ToString + ?Sized>(&self, label1: &L1, label2: &L2) -> T::M {
        self.with_label_values(&[label1.to_string().as_str(), label2.to_string().as_str()])
    }
}
