//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use axum_extra::headers::authorization::Bearer;
use tari_ootle_walletd_client::{
    permissions::{Crud, Permission},
    types::{TrustedValidatorSetInfo, ValidatorSetGetStatusRequest, ValidatorSetGetStatusResponse},
};

use crate::handlers::HandlerContext;

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
    Ok(ValidatorSetGetStatusResponse {
        indexer_trust: sdk.sdk_config().indexer_trust,
        trusted,
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
    }
}
