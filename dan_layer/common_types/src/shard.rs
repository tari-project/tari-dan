//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Display, ops::RangeInclusive};

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::{substate_address::END_SHARD_MAX, uint::U256, SubstateAddress};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct Shard(#[cfg_attr(feature = "ts", ts(type = "number"))] u32);

impl Shard {
    pub fn as_u32(self) -> u32 {
        self.0
    }

    pub fn to_substate_address_range(self, num_shards: u32) -> RangeInclusive<SubstateAddress> {
        if num_shards == 0 {
            return RangeInclusive::new(SubstateAddress::zero(), SubstateAddress::max());
        }

        // There will never be close to 2^31-1 committees but the calculation below will overflow/panic if
        // num_shards.leading_zeros() == 0.
        let num_shards = num_shards.min(crate::substate_address::MAX_NUM_SHARDS);

        if num_shards.is_power_of_two() {
            let shard_size = END_SHARD_MAX >> num_shards.trailing_zeros();
            let start = U256::from(self.0) * shard_size;
            let end = if self.0 == num_shards - 1 {
                U256::MAX
            } else {
                start + shard_size - 1
            };
            return RangeInclusive::new(SubstateAddress::from_u256(start), SubstateAddress::from_u256(end));
        }

        // Round down to the next power of two.
        let num_shards_next_pow2 = num_shards.next_power_of_two();
        // Half the next power of two i.e. num_shards rounded down to previous power of two
        let num_shards_prev_pow2 = num_shards_next_pow2 >> 1;
        // The "extra" half shards in the space
        let num_half_shards = num_shards % num_shards_prev_pow2;

        let num_shards_next_pow2 = U256::from(num_shards_next_pow2);
        // Power of two division using bit shifts
        let half_shard_size = END_SHARD_MAX >> num_shards_next_pow2.trailing_zeros();
        let full_shard_size = END_SHARD_MAX >> num_shards_prev_pow2.trailing_zeros();

        let start = U256::from(self.0.min(num_half_shards * 2)) * half_shard_size +
            U256::from(self.0.saturating_sub(num_half_shards * 2)) * full_shard_size;

        let end = if self.0 == num_shards - 1 {
            // Any remainder when dividing the shards into the shard-space is added to the last shard
            U256::MAX
        } else if self.0 >= num_half_shards * 2 {
            start + full_shard_size - 1
        } else {
            start + half_shard_size - 1
        };

        RangeInclusive::new(SubstateAddress::from_u256(start), SubstateAddress::from_u256(end))
    }
}

impl From<u32> for Shard {
    fn from(v: u32) -> Self {
        Self(v)
    }
}

impl PartialEq<u32> for Shard {
    fn eq(&self, other: &u32) -> bool {
        self.0 == *other
    }
}
impl PartialEq<Shard> for u32 {
    fn eq(&self, other: &Shard) -> bool {
        *self == other.as_u32()
    }
}

impl Display for Shard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_u32())
    }
}

#[cfg(test)]
mod test {
    use std::iter;

    use indexmap::IndexMap;

    use super::*;
    use crate::uint::U256_ONE;

    #[test]
    fn committee_is_properly_computed() {
        let power_of_twos = iter::successors(Some(1), |x| Some(x * 2)).take(10);
        let mut split_map = IndexMap::<_, Vec<U256>>::new();
        for num_of_shards in power_of_twos {
            let mut previous_end = U256::ZERO;
            for shard_index in 0..num_of_shards {
                let shard = Shard::from(shard_index);
                let range = shard.to_substate_address_range(num_of_shards);
                if shard_index > 0 {
                    assert_eq!(
                        range.start().to_u256(),
                        previous_end + U256_ONE,
                        "Bucket should start where the previous one ended+1"
                    );
                }
                split_map.entry(num_of_shards).or_default().push(range.end().to_u256());
                previous_end = range.end().to_u256();
            }
            assert_eq!(previous_end, U256::MAX, "Last bucket should end at U256::MAX");
        }

        let mut i = 0usize;
        for (num_of_shards, splits) in &split_map {
            // Each split in the next num_of_shards should match the previous shard splits
            let Some(next_splits) = split_map.get(&(num_of_shards << 1)) else {
                break;
            };

            i += 1;

            for (split, next_split) in splits.iter().zip(
                next_splits
                    .iter()
                    .enumerate()
                    // Every 2nd boundary matches
                    .filter(|(i, _)| i % 2 == 1)
                    .map(|(_, s)| s),
            ) {
                assert_eq!(*split, *next_split, "Bucket should end where the next one starts-1");
            }
        }

        // Check that we didnt break early
        assert_eq!(i, 9);
    }
}
