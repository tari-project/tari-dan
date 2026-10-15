//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Rust macros that can be used inside templates

pub use tari_template_abi::rust;

pub use crate::types::LogLevel;

/// Macro for emitting log messages from inside templates
#[macro_export]
macro_rules! log {
    ($lvl:expr, $fmt:expr) => {
        $crate::engine().emit_log($lvl, $crate::macros::rust::format!($fmt))
    };
    ($lvl:expr, $fmt:expr, $($args:tt)*) => {
        $crate::engine().emit_log($lvl, $crate::macros::rust::format!($fmt, $($args)*))
    };
}

/// Macro for emitting debug log messages from inside templates
#[macro_export]
macro_rules! debug {
    ($fmt:expr) => {
        $crate::log!($crate::macros::LogLevel::Debug, $fmt)
    };
    ($fmt:expr, $($args:tt)*) => {
        $crate::log!($crate::macros::LogLevel::Debug, $fmt, $($args)*)
    };
}

/// Macro for emitting log messages from inside templates
#[macro_export]
macro_rules! info {
    ($fmt:expr) => {
        $crate::log!($crate::macros::LogLevel::Info, $fmt)
    };
    ($fmt:expr, $($args:tt)*) => {
        $crate::log!($crate::macros::LogLevel::Info, $fmt, $($args)*)
    };
}

/// Macro for emitting warn log messages from inside templates
#[macro_export]
macro_rules! warn {
    ($fmt:expr) => {
        $crate::log!($crate::macros::LogLevel::Warn, $fmt)
    };
    ($fmt:expr, $($args:tt)*) => {
        $crate::log!($crate::macros::LogLevel::Warn, $fmt, $($args)*)
    };
}

/// Macro for emitting error log messages from inside templates
#[macro_export]
macro_rules! error {
    ($fmt:expr) => {
        $crate::log!($crate::macros::LogLevel::Error, $fmt)
    };
    ($fmt:expr, $($args:tt)*) => {
        $crate::log!($crate::macros::LogLevel::Error, $fmt, $($args)*)
    };
}
