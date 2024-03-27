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

/// Macro for emitting log messages from inside templates
#[macro_export]
macro_rules! log {
    ($lvl:expr, $fmt:expr) => {
        $crate::engine::engine().emit_log($lvl, $fmt)
    };
    ($lvl:expr, $fmt:expr, $($args:tt)*) => {
        $crate::engine::engine().emit_log($lvl, format!($fmt, $($args)*))
    };
}

/// Macro for emitting log messages from inside templates
#[macro_export]
macro_rules! info {
    ($fmt:expr) => {
        $crate::macros::log!($crate::args::LogLevel::Info, $fmt)
    };
    ($fmt:expr, $($args:tt)*) => {
        $crate::macros::log!($crate::args::LogLevel::Info, $fmt, $($args)*)
    };
}

/// Macro for emitting warn log messages from inside templates
#[macro_export]
macro_rules! warn {
    ($fmt:expr) => {
        $crate::macros::log!($crate::args::LogLevel::Warn, $fmt)
    };
    ($fmt:expr, $($args:tt)*) => {
        $crate::macros::log!($crate::args::LogLevel::Warn, $fmt, $($args)*)
    };
}

/// Macro for emitting error log messages from inside templates
#[macro_export]
macro_rules! error {
    ($fmt:expr) => {
        $crate::macros::log!($crate::args::LogLevel::Error, $fmt)
    };
    ($fmt:expr, $($args:tt)*) => {
        $crate::macros::log!($crate::args::LogLevel::Error, $fmt, $($args)*)
    };
}
