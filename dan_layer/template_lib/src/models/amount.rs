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

use std::{
    fmt::{Display, Formatter},
    num::TryFromIntError,
};

use newtype_ops::newtype_ops;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Amount(pub i64);

impl Amount {
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

    pub fn checked_add(&self, other: &Self) -> Option<Self> {
        self.0.checked_add(other.0).map(Amount)
    }

    pub fn saturating_add(&self, other: &Self) -> Self {
        Amount(self.0.saturating_add(other.0))
    }

    pub fn checked_sub(&self, other: &Self) -> Option<Self> {
        self.0.checked_sub(other.0).map(Amount)
    }

    pub fn saturating_sub(&self, other: &Self) -> Self {
        Amount(self.0.saturating_sub(other.0))
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
}

impl TryFrom<u64> for Amount {
    type Error = TryFromIntError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Ok(Amount(i64::try_from(value)?))
    }
}

// TODO: This is fallible since changing from i128 to i64
impl From<usize> for Amount {
    fn from(value: usize) -> Self {
        Amount::new(value as i64)
    }
}

impl From<i32> for Amount {
    fn from(value: i32) -> Self {
        Amount::new(i64::from(value))
    }
}

impl From<u32> for Amount {
    fn from(value: u32) -> Self {
        Amount::new(i64::from(value))
    }
}
impl From<i64> for Amount {
    fn from(value: i64) -> Self {
        Amount::new(value)
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
        let a = Amount::new(4);
        let b = Amount::new(6);
        let c = a + b;
        assert_eq!(c, 10);
        let d = a - b;
        assert_eq!(d, -2);
        let e = a * b;
        assert_eq!(e, 24);
        let f = b / a;
        assert_eq!(f, 1);
    }

    #[test]
    fn can_serialize() {
        let a = Amount::new(4);
        let b = serde_json::to_string(&a).unwrap();
        assert_eq!(b, "4");
    }
}
