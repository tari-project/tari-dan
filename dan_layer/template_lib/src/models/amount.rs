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

use core::ops::{Add, AddAssign, Div, Mul, Sub, SubAssign};
use std::{num::ParseIntError, str::FromStr};

use ciborium::tag::Required;
use serde::{Deserialize, Serialize};
use tari_template_abi::rust::{
    fmt::{Display, Formatter},
    iter::Sum,
    num::TryFromIntError,
};

use super::BinaryTag;
const TAG: u64 = BinaryTag::Amount as u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Amount(pub Required<i64, TAG>);

impl Amount {
    pub const fn new(amount: i64) -> Self {
        Amount(Required::<i64, TAG>(amount))
    }

    pub const fn zero() -> Self {
        Amount::new(0)
    }

    pub fn is_zero(&self) -> bool {
        self.0 .0 == 0
    }

    pub fn is_positive(&self) -> bool {
        self.0 .0 >= 0
    }

    pub fn is_negative(&self) -> bool {
        !self.is_positive()
    }

    pub fn value(&self) -> i64 {
        self.0 .0
    }

    pub fn checked_add(&self, other: &Self) -> Option<Self> {
        self.0 .0.checked_add(other.0 .0).map(Amount::new)
    }

    pub fn saturating_add(&self, other: &Self) -> Self {
        Amount::new(self.0 .0.saturating_add(other.0 .0))
    }

    pub fn checked_sub(&self, other: Self) -> Option<Self> {
        self.0 .0.checked_sub(other.0 .0).map(Amount::new)
    }

    pub fn saturating_sub(&self, other: Self) -> Self {
        Amount::new(self.0 .0.saturating_sub(other.0 .0))
    }

    pub fn checked_sub_positive(&self, other: Self) -> Option<Self> {
        if self.is_negative() || other.is_negative() {
            return None;
        }
        if self < &other {
            return None;
        }

        Some(Amount::new(self.0 .0 - other.0 .0))
    }

    pub fn checked_mul(&self, other: &Self) -> Option<Self> {
        self.0 .0.checked_mul(other.0 .0).map(Amount::new)
    }

    pub fn saturating_mul(&self, other: &Self) -> Self {
        Amount::new(self.0 .0.saturating_mul(other.0 .0))
    }

    pub fn checked_div(&self, other: &Self) -> Option<Self> {
        self.0 .0.checked_div(other.0 .0).map(Amount::new)
    }

    pub fn saturating_div(&self, other: &Self) -> Self {
        Amount::new(self.0 .0.saturating_div(other.0 .0))
    }

    pub fn as_u64_checked(&self) -> Option<u64> {
        self.0 .0.try_into().ok()
    }
}

impl Default for Amount {
    fn default() -> Self {
        Self::new(0)
    }
}

impl TryFrom<u64> for Amount {
    type Error = TryFromIntError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Ok(Amount::new(i64::try_from(value)?))
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

// newtype_ops! { [Amount] {add sub mul div} {:=} Self Self }
// newtype_ops! { [Amount] {add sub mul div} {:=} &Self &Self }
// newtype_ops! { [Amount] {add sub mul div} {:=} Self &Self }
//
// newtype_ops! { [Amount] {add sub mul div} {:=} Self i64 }
// newtype_ops! { [Amount] {add sub mul div} {:=} &Self &i64 }
// newtype_ops! { [Amount] {add sub mul div} {:=} Self &i64 }

impl Add for Amount {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self::new(self.0 .0 + other.0 .0)
    }
}

impl Sub for Amount {
    type Output = Self;

    fn sub(self, other: Self) -> Self::Output {
        Self::new(self.0 .0 - other.0 .0)
    }
}

impl Mul for Amount {
    type Output = Self;

    fn mul(self, other: Self) -> Self {
        Self::new(self.0 .0 * other.0 .0)
    }
}

impl Div for Amount {
    type Output = Self;

    fn div(self, other: Self) -> Self {
        Self::new(self.0 .0 / other.0 .0)
    }
}

impl AddAssign for Amount {
    fn add_assign(&mut self, other: Amount) {
        self.0 .0 += other.0 .0;
    }
}

impl SubAssign for Amount {
    fn sub_assign(&mut self, other: Amount) {
        self.0 .0 -= other.0 .0;
    }
}

impl PartialEq<i64> for Amount {
    fn eq(&self, other: &i64) -> bool {
        self.0 .0 == *other
    }
}

impl Sum for Amount {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Amount::zero(), |a, b| a + b)
    }
}

impl Display for Amount {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0 .0)
    }
}

impl FromStr for Amount {
    type Err = ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Amount::new(s.parse()?))
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
        assert_eq!(b, "{\"@@TAGGED@@\":[0,4]}");
    }
}
