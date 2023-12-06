//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

//! Context definitions related to the caller of an instruction

use tari_template_abi::{call_engine, EngineOp};

use crate::{
    args::{CallerContextAction, CallerContextInvokeArg, InvokeResult},
    crypto::RistrettoPublicKeyBytes,
    models::ComponentAddress,
};

/// Allows a template to access information about the current instruction's caller
pub struct CallerContext;

impl CallerContext {
    /// Returns the  public key used to sign the transaction that is currently being executed
    pub fn transaction_signer_public_key() -> RistrettoPublicKeyBytes {
        let resp: InvokeResult = call_engine(EngineOp::CallerContextInvoke, &CallerContextInvokeArg {
            action: CallerContextAction::GetCallerPublicKey,
        });

        resp.decode().expect("Failed to decode PublicKey")
    }

    /// Returns the address of the component that is being called in the current instruction. 
    /// Assumes that the instruction is a call method; otherwise, it will panic
    pub fn current_component_address() -> ComponentAddress {
        let resp: InvokeResult = call_engine(EngineOp::CallerContextInvoke, &CallerContextInvokeArg {
            action: CallerContextAction::GetComponentAddress,
        });

        resp.decode::<Option<ComponentAddress>>()
            .expect("Failed to decode Option<ComponentAddress>")
            .expect("Not in a component instance context")
    }
}
