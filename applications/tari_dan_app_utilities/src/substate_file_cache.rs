// Copyright 2023. The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::path::PathBuf;
use async_trait::async_trait;
use tari_engine_types::substate::Substate;
use tari_bor::{encode, decode_exact};
use tari_indexer_lib::substate_cache::{SubstateCacheError, SubstateCache};

#[derive(Debug, Clone)]
pub struct SubstateFileCache {
    cache_dir_path: String
}

impl SubstateFileCache {
    pub fn new(path_buf: PathBuf) -> Result<Self, SubstateCacheError> {
        let cache_dir_path = path_buf
            .into_os_string()
            .into_string()
            .map_err(|_| SubstateCacheError("Invalid substate cache path".to_string()))?;

        Ok(Self {
            cache_dir_path
        })
    }
}

#[async_trait]
impl SubstateCache for SubstateFileCache {
    async fn read(self, address: String) -> Result<Option<Substate>, SubstateCacheError> {
        let res = cacache::read(&self.cache_dir_path, address).await;
        match res {
            Ok(value) => {
                // cache hit
                let substate = decode_exact::<Substate>(&value)
                    .map_err(|e| SubstateCacheError(e.to_string()))?;
                return Ok(Some(substate));
            },
            Err(e) => {
                // cache miss
                if let cacache::Error::EntryNotFound(_, _) = e {
                    return Ok(None);
                // cache error
                } else {
                    return Err(SubstateCacheError(format!("{}", e)))}
                }
        }
    }

    async fn write(self, address: String, substate: &Substate) -> Result<(), SubstateCacheError> {
        let encoded_substate = encode(&substate)
            .map_err(|e| SubstateCacheError(e.to_string()))?;
        cacache::write(&self.cache_dir_path, address, encoded_substate).await
            .map_err(|e| SubstateCacheError(format!("{}", e)))?;
        Ok(())
    }
}