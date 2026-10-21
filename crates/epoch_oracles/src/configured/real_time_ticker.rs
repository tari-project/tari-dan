//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    num::NonZeroU64,
    task::{Context, Poll, ready},
    time::Duration,
};

use tari_ootle_common_types::Epoch;
use tari_sidechain::FixedHash;

use crate::configured::{EpochTicker, EpochTickerData};

#[derive(Debug)]
pub struct RealTimeEpochTicker {
    base_time: time::OffsetDateTime,
    epoch_time_secs: NonZeroU64,
    initial_epoch: Epoch,
    interval: Option<tokio::time::Interval>,
    epoch: Epoch,
}

impl RealTimeEpochTicker {
    pub fn new(initial_epoch: Epoch, base_time: time::OffsetDateTime, start_epoch: Epoch) -> Self {
        Self {
            interval: Some({
                let mut interval = tokio::time::interval(Duration::from_secs(1));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
                interval
            }),
            epoch_time_secs: NonZeroU64::new(120).expect("Non zero"),
            initial_epoch,
            base_time,
            epoch: start_epoch,
        }
    }

    pub fn disable_ticks(mut self) -> Self {
        self.interval = None;
        self
    }

    pub fn with_epoch_time_secs(mut self, epoch_time_secs: NonZeroU64) -> Self {
        self.epoch_time_secs = epoch_time_secs;
        self
    }

    fn calc_current_epoch(&self) -> Epoch {
        let now = time::OffsetDateTime::now_utc();
        let elapsed_secs = (now - self.base_time).whole_seconds().max(0) as u64 / self.epoch_time_secs.get();
        Epoch(elapsed_secs)
    }

    fn increment_epoch(&mut self) -> Epoch {
        let epoch = self.epoch;
        self.epoch += Epoch(1);
        epoch
    }
}

impl EpochTicker for RealTimeEpochTicker {
    fn poll_tick(&mut self, cx: &mut Context) -> Poll<Option<EpochTickerData>> {
        let calculated_epoch = self.calc_current_epoch();

        // Emit quickly to the initial epoch
        if calculated_epoch < self.initial_epoch {
            let epoch = self.increment_epoch();
            let epoch_hash = calc_static_epoch_hash(epoch);
            return Poll::Ready(Some(EpochTickerData {
                epoch,
                epoch_hash,
                done_for_now: false,
            }));
        }

        // Emit quickly to catch up to the calculated epoch
        if calculated_epoch >= self.epoch {
            let epoch = self.increment_epoch();
            let epoch_hash = calc_static_epoch_hash(epoch);
            return Poll::Ready(Some(EpochTickerData {
                epoch,
                epoch_hash,
                // Catching up
                done_for_now: calculated_epoch == epoch,
            }));
        }

        if let Some(interval_mut) = self.interval.as_mut() {
            loop {
                // Every tick, check if we need to emit a new epoch
                ready!(interval_mut.poll_tick(cx));

                if calculated_epoch >= self.epoch {
                    let epoch = self.increment_epoch();
                    let epoch_hash = calc_static_epoch_hash(epoch);
                    return Poll::Ready(Some(EpochTickerData {
                        epoch,
                        epoch_hash,
                        // Catching up if calculated_epoch > self.epoch
                        done_for_now: calculated_epoch == epoch,
                    }));
                }
            }
        }
        // Ticks have been disabled, never return any new epochs
        assert!(self.interval.is_none(), "If interval is Some, we should not reach here");
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
    async fn it_ticks_according_to_relative_time() {
        let base_time = time::OffsetDateTime::now_utc();
        let mut ticker =
            RealTimeEpochTicker::new(Epoch(0), base_time, Epoch(0)).with_epoch_time_secs(1.try_into().unwrap());

        // done_for_now depends on scheduler latency against the wall clock, so only the epoch sequence is asserted
        for i in 0..5 {
            let res = timeout(Duration::from_secs(10), poll_fn(|cx| ticker.poll_tick(cx)))
                .await
                .expect("elapsed")
                .unwrap();
            assert_eq!(res.epoch, Epoch(i));
            assert_eq!(res.epoch_hash, calc_static_epoch_hash(Epoch(i)));
        }
    }

    #[tokio::test]
    async fn it_catches_up_to_current_epoch() {
        let base_time = time::OffsetDateTime::now_utc() - time::Duration::seconds(1000);
        let mut ticker =
            RealTimeEpochTicker::new(Epoch(500), base_time, Epoch(5)).with_epoch_time_secs(1.try_into().unwrap());

        for i in 5..1002 {
            let res = timeout(Duration::from_secs(10), poll_fn(|cx| ticker.poll_tick(cx))).await;
            assert_eq!(
                res,
                Ok(Some(EpochTickerData {
                    epoch: Epoch(i),
                    epoch_hash: calc_static_epoch_hash(Epoch(i)),
                    done_for_now: i >= 1000
                }))
            );
        }
    }

    mod config_test {
        use super::*;
        use crate::configured::Config;

        #[tokio::test]
        #[ignore = "Not for testing, use to simulate the result of a real config file."]
        async fn check_config_file() {
            let mut file = std::fs::File::open("../../data/ec.json").unwrap();
            let config = serde_json::from_reader::<_, Config>(&mut file).unwrap();
            let mut ticker = RealTimeEpochTicker::new(config.initial_epoch, config.base_time, Epoch(128))
                .with_epoch_time_secs(config.epoch_time.unwrap().as_secs().try_into().unwrap());
            let current = ticker.calc_current_epoch();
            eprintln!("current: {:?}", current);

            loop {
                let res = poll_fn(|cx| ticker.poll_tick(cx)).await;
                eprintln!("res: {:?}", res);
            }
        }
    }

    #[tokio::test]
    async fn it_stays_on_the_current_epoch_of_base_time_in_future() {
        let base_time = time::OffsetDateTime::now_utc() + time::Duration::seconds(1);
        let mut ticker =
            RealTimeEpochTicker::new(Epoch(0), base_time, Epoch(1)).with_epoch_time_secs(1.try_into().unwrap());

        for i in 0..3 {
            let res = timeout(Duration::from_secs(10), poll_fn(|cx| ticker.poll_tick(cx)))
                .await
                .expect("elapsed")
                .unwrap();
            assert_eq!(res.epoch, Epoch(i + 1));
        }
    }
}
