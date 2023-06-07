//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ciborium::value::Value;

pub fn walk_all<V, T>(value: &Value, visitor: &mut V) -> Result<(), V::Error>
where
    V: ValueVisitor<T>,
    T: FromTagAndValue<Error = V::Error>,
{
    match value {
        Value::Integer(_) => {},
        Value::Bytes(_) => {},
        Value::Float(_) => {},
        Value::Text(_) => {},
        Value::Bool(_) => {},
        Value::Null => {},
        Value::Tag(tag, val) => {
            let val = T::try_from_tag_and_value(*tag, val)?;
            visitor.visit(val)?;
        },
        Value::Array(values) => {
            for value in values {
                walk_all(value, visitor)?;
            }
        },
        Value::Map(value_pairs) => {
            for (key, value) in value_pairs {
                walk_all(key, visitor)?;
                walk_all(value, visitor)?;
            }
        },
        _ => {},
    }
    Ok(())
}

pub trait ValueVisitor<T> {
    type Error;

    fn visit(&mut self, value: T) -> Result<(), Self::Error>;
}

pub trait FromTagAndValue {
    type Error;

    fn try_from_tag_and_value(tag: u64, value: &Value) -> Result<Self, Self::Error>
    where
        Self: Sized;
}
