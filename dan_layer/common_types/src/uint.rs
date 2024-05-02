//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub type U256 = ethnum::U256;

pub const U256_ZERO: U256 = U256::from_words(0, 0);
pub const U256_ONE: U256 = U256::from_words(0, 1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u256_one() {
        assert_eq!(U256_ONE, U256::from(1u64));
    }
}
