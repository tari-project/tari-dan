//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::task::{Context, Poll, ready};

use tari_ootle_common_types::Epoch;
use tari_sidechain::FixedHash;

pub trait EpochTicker {
    fn poll_tick(&mut self, cx: &mut Context) -> Poll<Option<EpochTickerData>>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EpochTickerData {
    pub epoch: Epoch,
    pub epoch_hash: FixedHash,
    pub done_for_now: bool,
}

#[derive(Debug)]
pub struct IntervalEpochTicker {
    interval: Option<tokio::time::Interval>,
    initial_epoch: Epoch,
    epoch: Epoch,
}

impl IntervalEpochTicker {
    pub fn new(interval: Option<tokio::time::Duration>, initial_epoch: Epoch, current_epoch: Epoch) -> Self {
        Self {
            interval: interval.map(|d| {
                let mut interval = tokio::time::interval(d);
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
                interval
            }),
            initial_epoch,
            epoch: current_epoch,
        }
    }

    fn increment_epoch(&mut self) -> Epoch {
        let epoch = self.epoch;
        self.epoch += Epoch(1);
        epoch
    }
}

impl EpochTicker for IntervalEpochTicker {
    fn poll_tick(&mut self, cx: &mut Context) -> Poll<Option<EpochTickerData>> {
        // Tick quickly initially
        if self.epoch < self.initial_epoch {
            let epoch = self.increment_epoch();
            let epoch_hash = calc_static_epoch_hash(epoch);
            return Poll::Ready(Some(EpochTickerData {
                epoch,
                epoch_hash,
                done_for_now: false,
            }));
        }

        if let Some(interval_mut) = self.interval.as_mut() {
            ready!(interval_mut.poll_tick(cx));
            let epoch = self.increment_epoch();
            let epoch_hash = calc_static_epoch_hash(epoch);
            return Poll::Ready(Some(EpochTickerData {
                epoch,
                epoch_hash,
                done_for_now: true,
            }));
        }
        Poll::Pending
    }
}

fn calc_static_epoch_hash(epoch: Epoch) -> FixedHash {
    const U64_SIZE: usize = size_of::<u64>();
    const HASH_SIZE: usize = FixedHash::byte_size();
    let mut epoch_hash = [0u8; HASH_SIZE];
    epoch_hash[HASH_SIZE - U64_SIZE..].copy_from_slice(&epoch.to_be_bytes());
    epoch_hash.into()
}

#[cfg(test)]
mod tests {
    use std::{future::poll_fn, time::Duration};

    use tokio::time::timeout;

    use super::*;

    #[tokio::test]
    async fn it_ticks_according_to_interval() {
        let mut ticker = IntervalEpochTicker::new(Some(Duration::from_millis(10)), Epoch(0), Epoch(0));

        for i in 0..5 {
            let res = timeout(Duration::from_secs(1), poll_fn(|cx| ticker.poll_tick(cx))).await;
            assert_eq!(
                res,
                Ok(Some(EpochTickerData {
                    epoch: Epoch(i),
                    epoch_hash: calc_static_epoch_hash(Epoch(i)),
                    done_for_now: true
                }))
            );
        }
    }
}
