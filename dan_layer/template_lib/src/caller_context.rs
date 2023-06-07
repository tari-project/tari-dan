//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause
use tari_crypto::ristretto::RistrettoPublicKey;
use tari_template_abi::{call_engine, EngineOp};

use crate::args::{CallerContextAction, CallerContextInvokeArg, InvokeResult};

pub struct CallerContext {}

impl CallerContext {
    pub fn caller() -> RistrettoPublicKey {
        let resp: InvokeResult = call_engine(
            EngineOp::CallerContextInvoke,
            &CallerContextInvokeArg {
                action: CallerContextAction::GetCallerPublicKey,
            },
        );

        resp.decode().expect("Failed to decode PublicKey")
    }
}
