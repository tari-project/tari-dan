//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_bor::{decode, decode_exact, decode_from, encode_into, Decode, Encode};
use tari_template_abi::call_debug;

use crate::{component::OwnedValue, prelude::ComponentInterface};

pub fn encode_component<T: Encode + ComponentInterface>(component: &T) -> Vec<u8> {
    let owned_values = component.get_owned_values();
    let mut buf = Vec::with_capacity(512);
    encode_into(&owned_values, &mut buf).expect("Failed to encode component owned values");
    encode_into(&component, &mut buf).expect("Failed to encode component state");
    buf
}

pub fn decode_component<T: Decode>(mut state: &[u8]) -> T {
    let _owned_values: Vec<OwnedValue> = decode_from(&mut state).expect("Failed to decode component owned values");
    let component = decode_exact(&mut state).expect("Failed to decode component state");
    component
}
