//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use core::ops::{Deref, DerefMut};

use ciborium::tag::Required;
use serde::{de, ser, Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BorTag<T, const TAG: u64>(Required<T, TAG>);

impl<T, const TAG: u64> BorTag<T, TAG> {
    pub const fn new(t: T) -> Self {
        Self(Required(t))
    }

    pub fn inner(&self) -> &T {
        &self.0 .0
    }

    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.0 .0
    }

    pub fn into_inner(self) -> T {
        self.0 .0
    }
}

impl<'de, V: Deserialize<'de>, const TAG: u64> Deserialize<'de> for BorTag<V, TAG> {
    #[inline]
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if deserializer.is_human_readable() {
            let v = V::deserialize(deserializer)?;
            Ok(BorTag(Required(v)))
        } else {
            let v = Required::<V, TAG>::deserialize(deserializer)?;
            Ok(BorTag(v))
        }
    }
}

impl<V: Serialize, const TAG: u64> Serialize for BorTag<V, TAG> {
    #[inline]
    fn serialize<S: ser::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            V::serialize(&self.0 .0, serializer)
        } else {
            self.0.serialize(serializer)
        }
    }
}

impl<T, const TAG: u64> Deref for BorTag<T, TAG> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner()
    }
}

impl<T, const TAG: u64> DerefMut for BorTag<T, TAG> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner_mut()
    }
}

impl<T: AsRef<[u8]>, const TAG: u64> AsRef<[u8]> for BorTag<T, TAG> {
    fn as_ref(&self) -> &[u8] {
        self.inner().as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode_exact, encode};

    #[test]
    fn encoding() {
        let t = BorTag::<_, 123>::new(222u8);
        let e = encode(&t).unwrap();

        let t = Required::<_, 123>(222u8);
        let e2 = encode(&t).unwrap();

        assert_eq!(e, e2);

        let t = BorTag::<_, 123>::new(222u8);
        let a = serde_json::to_string(&t).unwrap();
        assert_eq!(a, "222");
        let b: BorTag<u8, 123> = serde_json::from_str(&a).unwrap();
        assert_eq!(*b, 222u8);
    }

    #[test]
    fn decoding() {
        let t = BorTag::<_, 123>::new(222u8);
        let e = encode(&t).unwrap();
        let o: BorTag<u8, 123> = decode_exact(&e).unwrap();
        assert_eq!(t, o);
    }
}
