//   Copyright 2023. The Tari Project
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

use async_graphql::SimpleObject;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tari_crypto::tari_utilities::hex::from_hex;

use crate::substate_storage_sqlite::sqlite_substate_store_factory::{
    SqliteSubstateStoreReadTransaction, SubstateStoreReadTransaction,
};

#[derive(SimpleObject, Deserialize, Serialize)]
pub struct Event {
    pub(crate) template_address: [u8; 32],
    pub(crate) tx_hash: [u8; 32],
    pub(crate) topic: String,
    pub(crate) payload: HashMap<String, String>,
}

pub async fn extract_event(
    tx: &mut SqliteSubstateStoreReadTransaction<'_>,
    template_address: String,
    tx_hash: String,
) -> Result<Vec<Event>, String> {
    let events = tx.get_events(template_address, tx_hash).map_err(|e| e.to_string())?;
    events
        .iter()
        .map(|event| {
            let mut template_address = [0u8; 32];
            let template_address_buff = from_hex(&event.template_address).map_err(|e| e.to_string())?;
            template_address.copy_from_slice(&template_address_buff);

            let mut tx_hash = [0u8; 32];
            let tx_hash_buffer = from_hex(&event.tx_hash).map_err(|e| e.to_string())?;
            tx_hash.copy_from_slice(&tx_hash_buffer);

            let payload = serde_json::from_str(event.payload.as_str()).map_err(|e| e.to_string())?;
            Ok(Event {
                template_address,
                tx_hash,
                topic: event.topic.clone(),
                payload,
            })
        })
        .collect::<Result<Vec<Event>, String>>()
}
