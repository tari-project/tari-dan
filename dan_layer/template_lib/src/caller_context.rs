//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause
use tari_template_abi::{call_engine, EngineOp};

use crate::{
    args::{CallerContextAction, CallerContextInvokeArg, InvokeResult},
    crypto::RistrettoPublicKeyBytes,
    models::ComponentAddress,
};

pub struct CallerContext;

impl CallerContext {
    pub fn transaction_signer_public_key() -> RistrettoPublicKeyBytes {
        let resp: InvokeResult = call_engine(EngineOp::CallerContextInvoke, &CallerContextInvokeArg {
            action: CallerContextAction::GetCallerPublicKey,
        });

        resp.decode().expect("Failed to decode PublicKey")
    }

    pub fn current_component_address() -> ComponentAddress {
        let resp: InvokeResult = call_engine(EngineOp::CallerContextInvoke, &CallerContextInvokeArg {
            action: CallerContextAction::GetComponentAddress,
        });

        resp.decode::<Option<ComponentAddress>>()
            .expect("Failed to decode Option<ComponentAddress>")
            .expect("Not in a component instance context")
    }
}
