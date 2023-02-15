//  Copyright 2022, The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{io, io::Write, str::FromStr};

use thiserror::Error;

pub struct Prompt {
    label: String,
    default: Option<String>,
    value: Option<String>,
}

impl Prompt {
    pub fn new(label: &str) -> Self {
        Self {
            label: label.into(),
            default: None,
            value: None,
        }
    }

    pub fn with_default<T: ToString>(mut self, default: T) -> Self {
        self.default = Some(default.to_string());
        self
    }

    pub fn with_value<T: ToString>(mut self, value: Option<T>) -> Self {
        self.value = value.map(|r| r.to_string());
        self
    }

    pub fn ask(self) -> Result<String, CommandError> {
        if let Some(response) = self.value {
            return Ok(response);
        }
        loop {
            match self.default.as_ref().filter(|s| !s.is_empty()) {
                Some(default) => {
                    println!("{} (Default: {})", self.label, default);
                },
                None => {
                    println!("{}", self.label);
                },
            }
            print!("> ");
            io::stdout().flush()?;
            let mut line_buf = String::new();
            io::stdin().read_line(&mut line_buf)?;
            println!();
            let trimmed = line_buf.trim();
            if trimmed.is_empty() {
                match self.default {
                    Some(default) => return Ok(default),
                    None => continue,
                }
            } else {
                return Ok(trimmed.to_string());
            }
        }
    }

    pub fn ask_parsed<T>(self) -> Result<T, CommandError>
    where
        T: FromStr,
        T::Err: ToString,
    {
        let resp = self.ask()?;
        let parsed = resp
            .parse()
            .map_err(|e: T::Err| CommandError::InvalidArgument(e.to_string()))?;
        Ok(parsed)
    }
}

#[derive(Debug, Error)]
#[allow(clippy::large_enum_variant)]
pub enum CommandError {
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
    #[error(transparent)]
    IoError(#[from] io::Error),
}
