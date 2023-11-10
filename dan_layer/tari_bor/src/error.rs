//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

#[cfg(not(feature = "std"))]
use alloc::{format, string::String};

#[derive(Debug, Clone)]
pub struct BorError(String);

impl BorError {
    pub fn new(str: String) -> Self {
        Self(str)
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

#[cfg(feature = "std")]
impl std::fmt::Display for BorError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for BorError {
    fn description(&self) -> &str {
        &self.0
    }
}

impl From<ciborium::value::Error> for BorError {
    fn from(value: ciborium::value::Error) -> Self {
        BorError(format!("{}", value))
    }
}
