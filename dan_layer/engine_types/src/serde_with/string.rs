//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize, Serializer};

pub fn serialize<S: Serializer, T: ToString + Serialize>(v: &T, s: S) -> Result<S::Ok, S::Error> {
    if s.is_human_readable() {
        s.serialize_str(&v.to_string())
    } else {
        v.serialize(s)
    }
}

pub fn deserialize<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr + DeserializeOwned,
    T::Err: std::fmt::Display,
{
    if d.is_human_readable() {
        let s = <String as Deserialize>::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    } else {
        T::deserialize(d)
    }
}

pub mod option {
    use super::*;

    pub fn serialize<S: Serializer, T: ToString + Serialize>(v: &Option<T>, s: S) -> Result<S::Ok, S::Error> {
        if s.is_human_readable() {
            match v {
                Some(v) => s.serialize_some(&v.to_string()),
                None => s.serialize_none(),
            }
        } else {
            v.serialize(s)
        }
    }

    pub fn deserialize<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: FromStr + DeserializeOwned,
        T::Err: std::fmt::Display,
    {
        if d.is_human_readable() {
            let s = <Option<String> as Deserialize>::deserialize(d)?;
            s.map(|s| s.parse()).transpose().map_err(serde::de::Error::custom)
        } else {
            Option::<T>::deserialize(d)
        }
    }
}
