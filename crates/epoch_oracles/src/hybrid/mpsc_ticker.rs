//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::task::{Context, Poll};

use tokio::sync::mpsc;

use crate::configured::{EpochTicker, EpochTickerData};

/// Builds an `EpochTicker` backed by an unbounded mpsc channel.
///
/// Previously this used `tokio::sync::watch`, which is latest-value-only — when the base-layer
/// scanner raced across several epoch boundaries in quick succession, every intermediate
/// `EpochChanged` was overwritten before the consumer polled, so only the final epoch's
/// `(epoch, epoch_hash)` ever reached the configured oracle (and therefore the epoch manager's
/// persisted `epochs` table). mpsc preserves every tick in order.
pub fn mpsc_ticker() -> (MpscEpochTicker, mpsc::UnboundedSender<EpochTickerData>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (MpscEpochTicker { rx }, tx)
}

pub struct MpscEpochTicker {
    rx: mpsc::UnboundedReceiver<EpochTickerData>,
}

impl MpscEpochTicker {
    pub fn new(rx: mpsc::UnboundedReceiver<EpochTickerData>) -> Self {
        Self { rx }
    }
}

impl EpochTicker for MpscEpochTicker {
    fn poll_tick(&mut self, cx: &mut Context) -> Poll<Option<EpochTickerData>> {
        self.rx.poll_recv(cx)
    }
}

#[cfg(test)]
mod tests {
    use std::future::poll_fn;

    use tari_ootle_common_types::Epoch;
    use tari_sidechain::FixedHash;

    use super::*;

    #[tokio::test]
    async fn it_picks_up_ticks() {
        let (mut ticker, sender) = mpsc_ticker();
        let data = EpochTickerData {
            epoch: Epoch(1),
            epoch_hash: FixedHash::default(),
            done_for_now: false,
        };
        sender.send(data.clone()).unwrap();
        let res = poll_fn(|cx| ticker.poll_tick(cx)).await;
        assert_eq!(res, Some(data));

        drop(sender);

        let res = poll_fn(|cx| ticker.poll_tick(cx)).await;
        assert_eq!(res, None, "Expected None after sender is dropped");
    }

    #[tokio::test]
    async fn it_delivers_every_tick_in_order() {
        // Regression: the previous watch-channel implementation collapsed rapid ticks into the
        // latest value, which caused consensus to stamp the wrong epoch hash into the next-epoch
        // genesis block when the base-layer scanner caught up across multiple epoch boundaries.
        let (mut ticker, sender) = mpsc_ticker();
        let ticks: Vec<_> = (1..=4)
            .map(|i| EpochTickerData {
                epoch: Epoch(i),
                epoch_hash: FixedHash::from([i as u8; 32]),
                done_for_now: false,
            })
            .collect();
        for tick in &ticks {
            sender.send(tick.clone()).unwrap();
        }
        for expected in &ticks {
            let got = poll_fn(|cx| ticker.poll_tick(cx)).await;
            assert_eq!(got.as_ref(), Some(expected));
        }
    }
}
