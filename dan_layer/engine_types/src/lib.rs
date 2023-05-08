//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

pub mod base_layer_hashing;
pub mod bucket;
pub mod commit_result;
pub mod confidential;
pub mod events;
pub mod fees;
pub mod hashing;
pub mod indexed_value;
pub mod instruction;
pub mod instruction_result;
pub mod logs;
pub mod non_fungible;
pub mod non_fungible_index;
pub mod resource;
pub mod resource_container;
pub mod substate;
pub mod vault;

mod template;
use std::{fmt, marker::PhantomData, str::FromStr};

use serde::{
    de,
    de::{MapAccess, Visitor},
    Deserialize,
    Deserializer,
};
pub use template::{calculate_template_binary_hash, TemplateAddress};

pub(crate) fn string_or_struct<'de, T, D, TErr>(deserializer: D) -> Result<T, D::Error>
where
    T: Deserialize<'de> + FromStr<Err = TErr>,
    D: Deserializer<'de>,
    TErr: fmt::Display,
{
    // This is a Visitor that forwards string types to T's `FromStr` impl and
    // forwards map types to T's `Deserialize` impl. The `PhantomData` is to
    // keep the compiler from complaining about T being an unused generic type
    // parameter. We need T in order to know the Value type for the Visitor
    // impl.
    struct StringOrStruct<T>(PhantomData<fn() -> T>);

    impl<'de, T, TErr> Visitor<'de> for StringOrStruct<T>
    where
        T: Deserialize<'de> + FromStr<Err = TErr>,
        TErr: fmt::Display,
    {
        type Value = T;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("string or map")
        }

        fn visit_str<E>(self, value: &str) -> Result<T, E>
        where E: de::Error {
            FromStr::from_str(value).map_err(|e| E::custom(e))
        }

        fn visit_map<M>(self, map: M) -> Result<T, M::Error>
        where M: MapAccess<'de> {
            // `MapAccessDeserializer` is a wrapper that turns a `MapAccess`
            // into a `Deserializer`, allowing it to be used as the input to T's
            // `Deserialize` implementation. T then deserializes itself using
            // the entries from the map visitor.
            Deserialize::deserialize(de::value::MapAccessDeserializer::new(map))
        }
    }

    deserializer.deserialize_any(StringOrStruct(PhantomData))
}
