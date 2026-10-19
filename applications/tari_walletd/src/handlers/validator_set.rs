//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use axum_extra::headers::authorization::Bearer;
use tari_ootle_walletd_client::{
    permissions::{Crud, Permission},
    types::{
        TrustedValidatorSetInfo,
        ValidatorSetGetStatusRequest,
        ValidatorSetGetStatusResponse,
        ValidatorSetReanchorInfo,
    },
};

use crate::handlers::HandlerContext;

/// The most re-anchors `validator_set.get_status` reports. The web UI's Validator Set page states this cap.
const MAX_REANCHORS_REPORTED: u64 = 10;

pub async fn handle_get_status(
    context: &HandlerContext,
    token: Option<&Bearer>,
    _req: ValidatorSetGetStatusRequest,
) -> Result<ValidatorSetGetStatusResponse, anyhow::Error> {
    context.authorize(token, &[Permission::Settings(Crud::Read)])?;
    let sdk = context.wallet_sdk();
    let trusted = sdk.validator_set_api().status()?.map(|status| TrustedValidatorSetInfo {
        epoch: status.latest.epoch(),
        validator_set_hash: *status.latest.validator_set_hash(),
        anchor_epoch: status.anchor.validator_set.epoch(),
        anchor_validator_set_hash: *status.anchor.validator_set.validator_set_hash(),
        anchor: status.anchor.anchor,
        anchored_at: status.anchor.anchored_at.assume_utc().unix_timestamp(),
        synced_at: status.synced_at,
        walk_failing_since: status.walk_failing_since,
    });
    let reanchors = sdk
        .validator_set_api()
        .reanchors(MAX_REANCHORS_REPORTED)?
        .into_iter()
        .map(|reanchor| ValidatorSetReanchorInfo {
            previous_epoch: reanchor.event.previous_epoch,
            epoch: reanchor.event.epoch,
            validator_set_hash: reanchor.event.validator_set_hash,
            reanchored_at: reanchor.reanchored_at.assume_utc().unix_timestamp(),
        })
        .collect();
    Ok(ValidatorSetGetStatusResponse {
        indexer_trust: sdk.sdk_config().indexer_trust,
        trusted,
        reanchors,
    })
}

#[cfg(test)]
mod tests {
    use tari_ootle_wallet_sdk::IndexerTrust;

    use super::*;
    use crate::handlers::test_support::TestDaemon;

    #[tokio::test]
    async fn a_wallet_that_has_not_anchored_reports_no_trusted_set() {
        let daemon = TestDaemon::start().await;
        let status = handle_get_status(&daemon.context, Some(&daemon.session), ValidatorSetGetStatusRequest {})
            .await
            .unwrap();
        assert_eq!(status.indexer_trust, IndexerTrust::Verify);
        assert!(status.trusted.is_none());
        assert!(status.reanchors.is_empty());
    }
}
