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
use tari_dan_common_types::{crypto::create_key_pair, NodeAddressable};
use tari_template_lib::{crypto::RistrettoPublicKeyBytes, models::NonFungibleAddress};
use tari_utilities::hex::Hex;

#[derive(Debug)]
pub struct KeyManager {
    pub keys_path: PathBuf,
}

impl KeyManager {
    pub fn init<P: AsRef<Path>>(base_path: P) -> anyhow::Result<Self> {
        let keys_path = base_path.as_ref().join("keys");
        fs::create_dir_all(&keys_path)?;
        Ok(Self { keys_path })
    }

    pub fn create(&self) -> anyhow::Result<KeyPair> {
        let (k, p) = create_key_pair();
        let is_active = self.count() == 0;
        fs::write(
            self.keys_path.join(format!("{}.json", p)),
            json!({"key": k.to_hex()}).to_string(),
        )?;
        if is_active {
            self.set_active_key(&p.to_hex())?;
        }
        Ok(KeyPair {
            secret_key: k,
            public_key: p,
            is_active,
        })
    }

    pub fn count(&self) -> usize {
        match fs::read_dir(&self.keys_path) {
            Ok(r) => r.count(),
            Err(_) => 0,
        }
    }

    pub fn all(&self) -> Vec<KeyPair> {
        let active_key = read_active_key(&self.keys_path);
        let dir = match fs::read_dir(&self.keys_path) {
            Ok(r) => r,
            Err(_) => return vec![],
        };
        dir.filter_map(|entry| {
            let entry = entry.ok()?;
            let key = read_key(entry.path()).ok()?;
            let public_key = PublicKey::from_secret_key(&key);
            Some(KeyPair {
                secret_key: key,
                is_active: active_key.as_ref().map(|a| a.public_key == public_key).unwrap_or(false),
                public_key,
            })
        })
        .collect()
    }

    pub fn get_active_key(&self) -> Option<KeyPair> {
        read_active_key(&self.keys_path)
    }

    pub fn set_active_key(&self, name: &str) -> anyhow::Result<()> {
        if !self.keys_path.join(format!("{}.json", name)).exists() {
            return Err(anyhow!("Key does not exist"));
        }
        fs::write(self.keys_path.join("active_key"), name)?;
        Ok(())
    }
}

fn read_active_key<P: AsRef<Path>>(base_dir: P) -> Option<KeyPair> {
    let active_key = fs::read_to_string(base_dir.as_ref().join("active_key")).ok()?;
    read_key(base_dir.as_ref().join(format!("{}.json", active_key)))
        .ok()
        .map(|key| KeyPair {
            public_key: PublicKey::from_secret_key(&key),
            secret_key: key,
            is_active: true,
        })
}

fn read_key<P: AsRef<Path>>(path: P) -> anyhow::Result<PrivateKey> {
    let file = fs::read_to_string(path)?;
    let val = json::from_str::<json::Value>(&file)?;
    let key = val
        .get("key")
        .and_then(|k| k.as_str())
        .ok_or_else(|| anyhow!("No key"))?;
    Ok(PrivateKey::from_hex(key)?)
}

#[derive(Debug, Clone)]
pub struct KeyPair {
    pub secret_key: PrivateKey,
    pub public_key: PublicKey,
    pub is_active: bool,
}

impl KeyPair {
    pub fn to_owner_token(&self) -> NonFungibleAddress {
        NonFungibleAddress::from_public_key(RistrettoPublicKeyBytes::from_bytes(self.public_key.as_bytes()).unwrap())
    }
}

impl Display for KeyPair {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.public_key.to_hex())
    }
}
