//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub use tari_template_abi::call_debug;

#[macro_export]
macro_rules! debug {
    ($fmt:expr) => {
        $crate::macros::call_debug($fmt)
    };
    ($fmt:expr, $($args:tt)*) => {
        $crate::macros::call_debug(format!($fmt, $($args)*))
    };
}
