//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    pin::Pin,
    task::{Context, Poll},
};

use async_stream::try_stream;
use axum::{
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
};
use bytes::{Bytes, BytesMut};
use futures::{Stream, StreamExt};
use log::*;
use tari_indexer_client::{
    protobuf,
    types::{GetUtxoUpdatesRequest, UtxoStateUpdateSet, WalletUtxoUpdate},
};
use tari_ootle_common_types::{StateVersion, shard::Shard};

use crate::{
    rest_api::{encoder::Encoder, error::ErrorResponse, streaming::encoding::MimeTypeEncoder},
    substate_manager::SubstateManager,
};

const LOG_TARGET: &str = "tari::indexer::rest_api::streaming::utxo_stream";

pub struct UtxoUpdateStream {
    content_type: HeaderValue,
    inner: Pin<Box<dyn Stream<Item = anyhow::Result<Bytes>> + Send>>,
}

impl UtxoUpdateStream {
    pub fn new(substate_manager: SubstateManager, request: GetUtxoUpdatesRequest, encoder: MimeTypeEncoder) -> Self {
        let content_type = encoder.media_type();
        let inner = try_stream! {
            let mut buffer = BytesMut::with_capacity(1024);
            for &(shard, state_version) in &request.shard_state_versions {
                debug!(
                    target: LOG_TARGET,
                    "Requesting UTXO updates for shard {} from state version {}",
                    shard,
                    state_version
                );

                let UtxoStateUpdateSet {
                    updates,
                    max_state_version,
                    max_epoch,
                    has_more,
                } = substate_manager
                    .get_utxo_updates(
                        request.resource_address,
                        request.from_epoch,
                        shard,
                        state_version,
                        request.unspent_only,
                        request.per_shard_limit,
                    )
                    .await
                    .map_err(anyhow::Error::from)?;

                // `max_state_version` is the version the read reached, and becomes the client's
                // resume point for the shard. Every update at or below it was either delivered here
                // or excluded by this request's `from_epoch`/`unspent_only` filters. A shard whose
                // read matched nothing still reports it when the read moved past the cursor, so that
                // the client does not read the excluded rows again on its next pass.
                if updates.is_empty() && max_state_version <= state_version {
                    debug!(target: LOG_TARGET, "No updates found for shard {}, moving to next shard", shard);
                    continue;
                }

                debug!(
                    target: LOG_TARGET,
                    "Received {} updates for shard {shard}, max_epoch = {max_epoch}, max_state_version {max_state_version} (has_more = {has_more})",
                    updates.len(),
                );

                let mut pending = PendingUpdates {
                    sos_emitted: false,
                    shard,
                    max_state_version,
                    has_more,
                    updates,
                    index: 0,
                };

                loop {
                    let mut payload = protobuf::UtxoUpdatePayload::default();

                    if !pending.sos_emitted {
                        payload.sos = Some(protobuf::StartOfShard {
                            shard: pending.shard.as_u32(),
                            max_state_version: pending.max_state_version.as_u64(),
                            num_updates: u32::try_from(pending.updates_len()).expect("loaded more than u32::MAX updates"),
                            has_more: pending.has_more,
                        });
                        pending.sos_emitted = true;
                    }

                    if let Some(update) = pending.next() {
                        payload.update = Some(wallet_utxo_update_to_protobuf(update));
                    }

                    if pending.is_empty() {
                        payload.eos = Some(protobuf::EndOfShard {
                            max_state_version: pending.max_state_version.as_u64(),
                        });
                    }

                    encoder.encode_into(&payload, &mut buffer)?;

                    debug!(
                        target: LOG_TARGET,
                        "Encoded payload size: {}, pending updates remaining in batch: {}",
                        buffer.len(),
                        pending.updates_len()
                    );

                    yield buffer.split().freeze();

                    if payload.eos.is_some() {
                        break;
                    }
                }
            }
        };

        Self {
            content_type,
            inner: Box::pin(inner),
        }
    }
}

impl Stream for UtxoUpdateStream {
    type Item = anyhow::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.poll_next_unpin(cx)
    }
}

impl IntoResponse for UtxoUpdateStream {
    fn into_response(self) -> Response {
        let content_type = self.content_type.clone();
        let stream_body = axum::body::Body::from_stream(self);

        axum::response::Response::builder()
            .header(header::CONTENT_TYPE, content_type)
            .body(stream_body)
            .map_err(ErrorResponse::anyhow)
            .into_response()
    }
}

struct PendingUpdates {
    pub sos_emitted: bool,
    pub shard: Shard,
    pub max_state_version: StateVersion,
    pub has_more: bool,
    pub updates: Vec<WalletUtxoUpdate>,
    pub index: usize,
}

impl PendingUpdates {
    pub fn is_done(&self) -> bool {
        self.index >= self.updates.len()
    }

    pub fn next(&mut self) -> Option<&WalletUtxoUpdate> {
        if self.is_done() {
            return None;
        }
        let update = &self.updates[self.index];
        self.index += 1;
        Some(update)
    }

    pub const fn updates_len(&self) -> usize {
        self.updates.len() - self.index
    }

    pub const fn is_empty(&self) -> bool {
        self.updates_len() == 0
    }
}

fn wallet_utxo_update_to_protobuf(update: &WalletUtxoUpdate) -> protobuf::WalletUtxoUpdate {
    match update {
        WalletUtxoUpdate::Unspent(unspent) => protobuf::WalletUtxoUpdate::Unspent(protobuf::UtxoUnspent {
            tag: unspent.tag.value(),
            public_nonce: unspent.public_nonce.to_vec(),
        }),
        WalletUtxoUpdate::Spent(spent) => protobuf::WalletUtxoUpdate::Spent(protobuf::UtxoSpent {
            id: spent.id.as_bytes().to_vec(),
            version: spent.version.as_u64(),
        }),
        WalletUtxoUpdate::Burnt(burnt) => protobuf::WalletUtxoUpdate::Burnt(protobuf::UtxoBurnt {
            id: burnt.id.as_bytes().to_vec(),
            version: burnt.version.as_u64(),
        }),
    }
}
