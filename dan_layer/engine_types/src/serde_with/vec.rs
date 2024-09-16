//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{de::DeserializeOwned, ser::SerializeSeq, Deserialize, Deserializer, Serialize, Serializer};

/// This gets around the JSON key must be a string issue by serializing the elements as a Vec containing the key and
/// value as a tuple
pub fn serialize<'a, S, I, K, V>(v: I, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    I: IntoIterator<Item = (&'a K, &'a V)> + Serialize,
    I::IntoIter: ExactSizeIterator,
    K: Serialize + 'a,
    V: Serialize + 'a,
{
    if !s.is_human_readable() {
        return v.serialize(s);
    }
    let iter = v.into_iter();
    let mut seq = s.serialize_seq(Some(iter.len()))?;
    for (k, v) in iter {
        seq.serialize_element(&(k, v))?;
    }
    seq.end()
}

pub fn deserialize<'de, D, T, K, V>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromIterator<(K, V)> + Deserialize<'de>,
    (K, V): DeserializeOwned,
{
    if !d.is_human_readable() {
        return T::deserialize(d);
    }

    let vec = Vec::<(K, V)>::deserialize(d)?;
    Ok(T::from_iter(vec))
}
