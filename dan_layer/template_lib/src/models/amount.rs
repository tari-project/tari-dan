//  Copyright 2022. The Tari Project
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

use std::cmp;

use newtype_ops::newtype_ops;
use serde::{Deserialize, Serialize};
use tari_template_abi::rust::{
    fmt::{Display, Formatter},
    iter::Sum,
    num::TryFromIntError,
};
#[cfg(feature = "ts")]
use ts_rs::TS;

/// Represents an integer quantity of any fungible or non-fungible resource
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(transparent)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct Amount(#[cfg_attr(feature = "ts", ts(type = "number"))] pub i64);

impl Amount {
    pub const MAX: Amount = Amount(i64::MAX);

    pub const fn new(amount: i64) -> Self {
        Amount(amount)
    }

    pub const fn zero() -> Self {
        Amount(0)
    }

    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }

    pub fn is_positive(&self) -> bool {
        self.0 >= 0
    }

    pub fn is_negative(&self) -> bool {
        !self.is_positive()
    }

    pub fn value(&self) -> i64 {
        self.0
    }

    pub fn checked_add(&self, other: Self) -> Option<Self> {
        self.0.checked_add(other.0).map(Amount)
    }

    pub fn saturating_add(&self, other: Self) -> Self {
        Amount(self.0.saturating_add(other.0))
    }

    pub fn checked_sub(&self, other: Self) -> Option<Self> {
        self.0.checked_sub(other.0).map(Amount)
    }

    pub fn saturating_sub(&self, other: Self) -> Self {
        Amount(self.0.saturating_sub(other.0))
    }

    pub fn saturating_sub_positive(&self, other: Self) -> Self {
        let amount = Amount(self.0 - other.0);
        if amount.is_negative() {
            Amount(0)
        } else {
            amount
        }
    }

    pub fn checked_sub_positive(&self, other: Self) -> Option<Self> {
        if self.is_negative() || other.is_negative() {
            return None;
        }
        if *self < other {
            return None;
        }

        Some(Amount(self.0 - other.0))
    }

    pub fn checked_mul(&self, other: &Self) -> Option<Self> {
        self.0.checked_mul(other.0).map(Amount)
    }

    pub fn saturating_mul(&self, other: &Self) -> Self {
        Amount(self.0.saturating_mul(other.0))
    }

    pub fn checked_div(&self, other: &Self) -> Option<Self> {
        self.0.checked_div(other.0).map(Amount)
    }

    pub fn saturating_div(&self, other: &Self) -> Self {
        Amount(self.0.saturating_div(other.0))
    }

    /// Returns the value as a u64 if possible, otherwise returns None.
    /// Since the internal representation is i64, this will return None if the value is negative.
    pub fn as_u64_checked(&self) -> Option<u64> {
        self.0.try_into().ok()
    }
}

impl TryFrom<u64> for Amount {
    type Error = TryFromIntError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Ok(Amount(i64::try_from(value)?))
    }
}

impl TryFrom<usize> for Amount {
    type Error = TryFromIntError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        Ok(Amount(i64::try_from(value)?))
    }
}

impl From<i32> for Amount {
    fn from(value: i32) -> Self {
        Amount(i64::from(value))
    }
}

impl From<u32> for Amount {
    fn from(value: u32) -> Self {
        Amount(i64::from(value))
    }
}
impl From<i64> for Amount {
    fn from(value: i64) -> Self {
        Amount(value)
    }
}

newtype_ops! { [Amount] {add sub mul div} {:=} Self Self }
newtype_ops! { [Amount] {add sub mul div} {:=} &Self &Self }
newtype_ops! { [Amount] {add sub mul div} {:=} Self &Self }

newtype_ops! { [Amount] {add sub mul div} {:=} Self i64 }
newtype_ops! { [Amount] {add sub mul div} {:=} &Self &i64 }
newtype_ops! { [Amount] {add sub mul div} {:=} Self &i64 }

impl PartialEq<i64> for Amount {
    fn eq(&self, other: &i64) -> bool {
        self.0 == *other
    }
}

impl PartialEq<u64> for Amount {
    fn eq(&self, other: &u64) -> bool {
        if self.is_negative() {
            return false;
        }
        self.0 as u64 == *other
    }
}

impl PartialOrd<u64> for Amount {
    fn partial_cmp(&self, other: &u64) -> Option<cmp::Ordering> {
        match i64::try_from(*other) {
            Ok(other) => self.0.partial_cmp(&other),
            Err(_) => Some(cmp::Ordering::Less),
        }
    }
}

impl PartialEq<Amount> for u64 {
    fn eq(&self, other: &Amount) -> bool {
        if other.is_negative() {
            return false;
        }
        *self == other.0 as u64
    }
}

impl PartialOrd<Amount> for u64 {
    fn partial_cmp(&self, other: &Amount) -> Option<cmp::Ordering> {
        match i64::try_from(*self) {
            Ok(v) => v.partial_cmp(&other.0),
            Err(_) => Some(cmp::Ordering::Greater),
        }
    }
}

impl Sum for Amount {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.map(|a| a.value()).sum()
    }
}

impl Sum<i64> for Amount {
    fn sum<I: Iterator<Item = i64>>(iter: I) -> Self {
        Self(iter.sum())
    }
}

impl Display for Amount {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_arithmetic() {
        let a = Amount(4);
        let b = Amount(6);
        let c = a + b;
        assert_eq!(c, 10i64);
        let d = a - b;
        assert_eq!(d, -2i64);
        let e = a * b;
        assert_eq!(e, 24i64);
        let f = b / a;
        assert_eq!(f, 1i64);
    }

    #[test]
    fn can_serialize() {
        let a = Amount(4);
        let b = serde_json::to_string(&a).unwrap();
        assert_eq!(b, "4");
    }

    #[test]
    fn u64_ord() {
        let a = Amount(4);
        let b = 6;
        assert!(a < b);
        assert!(b > a);
        assert!(a <= b);
        assert!(b >= a);

        // Negatives
        let c = Amount(-4);
        let d = 6;
        assert!(c < d);
        assert!(d > c);
        assert!(c <= d);
        assert!(d >= c);

        // Overflow
        let e = Amount(i64::MAX);
        let f = u64::MAX;
        assert!(e < f);
        assert!(f > e);
        assert!(e <= f);
        assert!(f >= e);
    }
}
