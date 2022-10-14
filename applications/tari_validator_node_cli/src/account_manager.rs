//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{
    fmt::{Display, Formatter},
    fs,
    path::{Path, PathBuf},
};

use anyhow::anyhow;
use serde_json as json;
use serde_json::json;
use tari_common_types::types::{PrivateKey, PublicKey};
use tari_crypto::keys::PublicKey as PublicKeyT;
use tari_dan_engine::crypto::create_key_pair;
use tari_utilities::hex::Hex;

#[derive(Debug)]
pub struct AccountFileManager {
    pub accounts_path: PathBuf,
}

impl AccountFileManager {
    pub fn init(base_path: PathBuf) -> anyhow::Result<Self> {
        let accounts_path = base_path.join("accounts");
        fs::create_dir_all(&accounts_path)?;
        Ok(Self { accounts_path })
    }

    pub fn create_account(&self) -> anyhow::Result<Account> {
        let (k, p) = create_key_pair();
        let is_active = self.count() == 0;
        fs::write(
            self.accounts_path.join(format!("{}.json", p)),
            json!({"key": k.to_hex()}).to_string(),
        )?;
        if is_active {
            self.set_active_account(&p.to_hex())?;
        }
        Ok(Account {
            secret_key: k,
            public_key: p,
            is_active,
        })
    }

    pub fn count(&self) -> usize {
        match fs::read_dir(&self.accounts_path) {
            Ok(r) => r.count(),
            Err(_) => 0,
        }
    }

    pub fn all(&self) -> Vec<Account> {
        let active_account = read_active_account(&self.accounts_path);
        let dir = match fs::read_dir(&self.accounts_path) {
            Ok(r) => r,
            Err(_) => return vec![],
        };
        dir.filter_map(|entry| {
            let entry = entry.ok()?;
            let key = read_account_key(&entry.path()).ok()?;
            let public_key = PublicKey::from_secret_key(&key);
            Some(Account {
                secret_key: key,
                is_active: active_account
                    .as_ref()
                    .map(|a| a.public_key == public_key)
                    .unwrap_or(false),
                public_key,
            })
        })
        .collect()
    }

    pub fn get_active_account(&self) -> Option<Account> {
        read_active_account(&self.accounts_path)
    }

    pub fn set_active_account(&self, name: &str) -> anyhow::Result<()> {
        if !self.accounts_path.join(format!("{}.json", name)).exists() {
            return Err(anyhow!("Account does not exist"));
        }
        fs::write(self.accounts_path.join("active_account"), name)?;
        Ok(())
    }
}

fn read_active_account<P: AsRef<Path>>(base_dir: P) -> Option<Account> {
    let active_account = fs::read_to_string(base_dir.as_ref().join("active_account")).ok()?;
    read_account_key(base_dir.as_ref().join(format!("{}.json", active_account)))
        .ok()
        .and_then(|key| {
            Some(Account {
                public_key: PublicKey::from_secret_key(&key),
                secret_key: key,
                is_active: true,
            })
        })
}

fn read_account_key<P: AsRef<Path>>(path: P) -> anyhow::Result<PrivateKey> {
    let file = fs::read_to_string(path)?;
    let val = json::from_str::<json::Value>(&file)?;
    let key = val
        .get("key")
        .and_then(|k| k.as_str())
        .ok_or_else(|| anyhow!("No key"))?;
    Ok(PrivateKey::from_hex(key)?)
}

#[derive(Debug, Clone)]
pub struct Account {
    pub secret_key: PrivateKey,
    pub public_key: PublicKey,
    pub is_active: bool,
}

impl Display for Account {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.public_key.to_hex())
    }
}
