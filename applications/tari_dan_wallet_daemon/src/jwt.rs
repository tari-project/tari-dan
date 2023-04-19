//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct Jwt {
    pub expiration: Duration,
    pub secret_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    exp: usize,
}

impl Jwt {
    pub fn new(expiration: Duration, secret_key: String) -> Self {
        Jwt { expiration, secret_key }
    }

    pub fn generate(&self) -> Result<String> {
        let my_claims = Claims {
            exp: (SystemTime::now() + self.expiration)
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as usize,
        };
        encode(
            &Header::default(),
            &my_claims,
            &EncodingKey::from_secret(self.secret_key.as_ref()),
        )
        .map_err(anyhow::Error::new)
    }

    pub fn check(&self, token: &str) -> Result<()> {
        decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.secret_key.as_ref()),
            &Validation::default(),
        )?;
        Ok(())
    }
}
