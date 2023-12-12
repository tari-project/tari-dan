//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Rust macros that can be used inside templates

pub use tari_template_abi::call_debug;

/// Macro for writing debug messages from inside templates
#[macro_export]
macro_rules! debug {
    ($fmt:expr) => {
        $crate::macros::call_debug($fmt)
    };
    ($fmt:expr, $($args:tt)*) => {
        $crate::macros::call_debug(format!($fmt, $($args)*))
    };
}
