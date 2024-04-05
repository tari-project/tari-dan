//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod definition;
mod minotari_node;

pub use definition::*;

use crate::config::InstanceType;

pub fn get_definition(instance_type: InstanceType) -> Box<dyn ProcessDefinition + 'static> {
    match instance_type {
        InstanceType::MinoTariNode => Box::new(minotari_node::MinotariNode::new()),
        _ => unimplemented!(),
    }
}
