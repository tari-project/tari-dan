//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_abi::rust::{
    fmt::{Display, Formatter},
    string::String,
};

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidByteLengthError {
    pub(super) size: usize,
    pub(super) expected: usize,
}

impl InvalidByteLengthError {
    pub fn actual_size(&self) -> usize {
        self.size
    }

    pub fn to_error_string(&self) -> String {
        format!(
            "Invalid byte length. Expected {} bytes, got {}",
            self.expected, self.size
        )
    }
}

impl Display for InvalidByteLengthError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_error_string())
    }
}

#[cfg(feature = "std")]
impl std::error::Error for InvalidByteLengthError {}
