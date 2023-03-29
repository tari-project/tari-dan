//  Copyright 2023, The Tari Project
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

use anyhow::Context;
use tari_engine_types::substate::{Substate, SubstateValue};

pub type JsonValue = serde_json::Value;
pub type JsonObject = serde_json::Map<String, JsonValue>;
pub type CborValue = ciborium::value::Value;

pub fn decode_substate_into_json(substate: &Substate) -> Result<JsonValue, anyhow::Error> {
    let substate_cbor = decode_into_cbor(&substate.to_bytes())?;
    let mut result = serde_json::to_value(substate_cbor)?;

    let substate_field = get_mut_json_field(&mut result, "substate")?;
    if let SubstateValue::NonFungible(s) = substate.substate_value() {
        if let Some(nf) = s.contents() {
            // let non_fungible_field = substate_field.as_object_mut().context("invalid field")?;
            let non_fungible_field = get_mut_json_field(substate_field, "NonFungible")?;
            let non_fungible_object = json_value_as_object(non_fungible_field)?;

            let data = decode_into_cbor(nf.data())?;
            let data_json = serde_json::to_value(data)?;
            non_fungible_object.insert("data".to_owned(), data_json);

            let mutable_data = decode_into_cbor(nf.mutable_data())?;
            let mutable_data_json = serde_json::to_value(mutable_data)?;
            non_fungible_object.insert("mutable_data".to_owned(), mutable_data_json);
        }
    }

    Ok(result)
}

fn get_mut_json_field<'a>(value: &'a mut JsonValue, field_name: &str) -> Result<&'a mut JsonValue, anyhow::Error> {
    let json_field = json_value_as_object(value)?
        .get_mut(field_name)
        .context("field does not exist")?;

    Ok(json_field)
}

fn json_value_as_object(value: &mut JsonValue) -> Result<&mut JsonObject, anyhow::Error> {
    let json_object = value.as_object_mut().context("invalid object")?;

    Ok(json_object)
}

fn decode_into_cbor(bytes: &[u8]) -> Result<CborValue, anyhow::Error> {
    Ok(ciborium::de::from_reader::<CborValue, _>(bytes)?)
}
