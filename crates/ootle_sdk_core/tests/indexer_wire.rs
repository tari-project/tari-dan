//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! The core parses indexer responses as `tari_indexer_client` serialises them.

use std::time::Duration;

use ootle_sdk_core::{DecodedSubstate, decode_substate, parse_finalized_result, types::result::TransactionOutcome};
use tari_consensus_types::Decision;
use tari_engine_types::{
    commit_result::{ExecuteResult, FinalizeResult, TransactionResult},
    encoded::Encoded,
    fees::FeeReceipt,
    resource_container::ResourceContainer,
    substate::{SubstateDiff, SubstateValue},
    vault::Vault,
};
use tari_indexer_client::types::{GetSubstateResponse, IndexerTransactionFinalizedResult, SubstateState};
use tari_ootle_common_types::SubstateVersion;
use tari_template_lib_types::{Hash32, constants::TARI_TOKEN};

#[test]
fn a_finalized_result_as_the_indexer_serves_it_parses() {
    let execute_result = ExecuteResult {
        finalize: FinalizeResult::new(
            Hash32::from_array([7; 32]),
            vec![],
            vec![],
            TransactionResult::Accept(SubstateDiff::new()),
            FeeReceipt::default(),
        ),
        execution_time: Duration::from_secs(1),
        execute_epoch: None,
        wasm_execution_points: 0,
        native_execution_points: 0,
    };
    let served = serde_json::to_string(&IndexerTransactionFinalizedResult::Finalized {
        final_decision: Decision::Commit,
        execution_result: Some(Encoded::encode(&execute_result).unwrap()),
        execution_time: Duration::from_secs(1),
        finalized_time: time::PrimitiveDateTime::MIN,
        abort_details: None,
    })
    .unwrap();

    let parsed = parse_finalized_result(&served).unwrap();

    assert!(matches!(parsed.submit.outcome, Some(TransactionOutcome::Commit)));
}

#[test]
fn a_substate_as_the_indexer_serves_it_decodes() {
    let value = SubstateValue::Vault(Vault::new(ResourceContainer::public_fungible(TARI_TOKEN, 100u64)));
    let served = serde_json::to_value(GetSubstateResponse {
        version: SubstateVersion::new(1),
        state: SubstateState::Up {
            substate: Encoded::encode(&value).unwrap(),
        },
        verified: true,
        proof: None,
    })
    .unwrap();

    let decoded = decode_substate(&served["state"]["up"]["substate"]).unwrap();

    assert!(
        matches!(decoded, DecodedSubstate::Vault {
            revealed_balance: 100,
            ..
        }),
        "{decoded:?}"
    );
}
