//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{borrow::Borrow, fmt::Display};

use tari_dan_engine::runtime::{ActionIdent, RuntimeError};
use tari_engine_types::{commit_result::RejectReason, resource_container::ResourceError};

pub fn assert_reject_reason<B: Borrow<RejectReason>, E: Display>(reason: B, error: E) {
    match reason.borrow() {
        // TODO: Would be great if we could enumerate specific reasons from within the engine rather than simply
        //       turning RuntimeError into a string
        RejectReason::ExecutionFailure(s) if s.contains(&error.to_string()) => {},
        r => panic!("Expected reject reason \"{}\" but got \"{}\"", error, r),
    }
}

#[allow(dead_code)]
pub fn assert_access_denied_for_action<B: Borrow<RejectReason>, A: Into<ActionIdent>>(reason: B, action_ident: A) {
    assert_reject_reason(reason, RuntimeError::AccessDenied {
        action_ident: action_ident.into(),
    })
}

#[allow(dead_code)]
pub fn assert_insufficient_funds_for_action<B: Borrow<RejectReason>>(reason: B) {
    assert_reject_reason(
        reason,
        RuntimeError::ResourceError(ResourceError::InsufficientBalance {
            details: "Bucket contained insufficient funds".to_string(),
        }),
    )
}
