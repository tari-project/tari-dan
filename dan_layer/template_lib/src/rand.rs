//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use tari_template_abi::{call_engine, EngineOp};

use crate::args::{GenerateRandomAction, GenerateRandomInvokeArg, InvokeResult};

pub fn random_bytes(len: u32) -> Vec<u8> {
    let resp: InvokeResult = call_engine(EngineOp::GenerateRandomInvoke, &GenerateRandomInvokeArg {
        action: GenerateRandomAction::GetRandomBytes { len },
    });
    resp.decode().expect("Failed to decode random bytes")
}

pub fn random_u32() -> u32 {
    let resp: InvokeResult = call_engine(EngineOp::GenerateRandomInvoke, &GenerateRandomInvokeArg {
        action: GenerateRandomAction::GetRandomBytes { len: 4 },
    });
    let v: Vec<u8> = resp.decode().expect("Failed to decode random u32");
    u32::from_le_bytes(v.as_slice().try_into().unwrap())
}
