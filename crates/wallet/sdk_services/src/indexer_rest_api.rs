//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use anyhow::anyhow;
use futures::{StreamExt, TryStreamExt};
use log::warn;
use reqwest::{IntoUrl, Url};
use tari_engine_types::{
    Utxo,
    substate::{Substate, SubstateId},
};
use tari_indexer_client::{
    error::IndexerRestClientError,
    event::{IndexerEvent, TransactionFinalizedEvent},
    protobuf,
    rest_api_client::IndexerRestApiClient,
    types::{
        GetSubstateRequest,
        GetSubstatesRequest,
        GetTransactionResultRequest,
        GetUtxoUpdatesRequest,
        GetUtxosRequest,
        IndexerTransactionFinalizedResult,
        ListWatchedSubstatesRequest,
        SubmitTransactionRequest,
        WatchedSubstateItem,
    },
};
use tari_ootle_common_types::{
    Epoch,
    StateVersion,
    array_utils::copy_fixed_checked,
    displayable::Displayable,
    optional::IsNotFoundError,
    response_status::{ResponseErrorStatus, TransactionStatusResponseError},
    shard::Shard,
};
use tari_ootle_transaction::{Transaction, TransactionEnvelope, TransactionId};
use tari_ootle_wallet_sdk::{
    models::{EndOfShard, StartOfShard, UtxoBurnt, UtxoSpent, UtxoUnspent, UtxoUpdatePayload, WalletUtxoUpdate},
    network::{
        SubstateQueryResult,
        TransactionFinalizedNotification,
        TransactionFinalizedResult,
        TransactionFinalizedStream,
        TransactionQueryResult,
        UtxoUpdateStream,
        WalletNetworkInterface,
    },
};
use tari_template_lib_types::{
    ResourceAddress,
    TemplateAddress,
    UtxoId,
    crypto::{RistrettoPublicKeyBytes, UtxoTag},
};
use time::{OffsetDateTime, PrimitiveDateTime};
use url::ParseError;

const LOG_TARGET: &str = "tari::ootle::wallet_services::indexer_rest_api";
const INVALID_REQUEST_CODE: i64 = 400;

#[derive(Debug, Clone)]
pub struct IndexerRestApiNetworkInterface {
    url: Arc<Mutex<Url>>,
}

impl IndexerRestApiNetworkInterface {
    pub fn new<T: IntoUrl>(url: T) -> Self {
        Self {
            url: Arc::new(Mutex::new(url.into_url().expect("Malformed indexer JSON-RPC address"))),
        }
    }

    fn get_client(&self) -> Result<IndexerRestApiClient, IndexerRestApiNetworkInterfaceError> {
        let client = IndexerRestApiClient::connect((*self.url.lock().unwrap()).clone())?;
        Ok(client)
    }

    pub fn set_endpoint(&self, endpoint: Url) -> &Self {
        *self.url.lock().unwrap() = endpoint;
        self
    }

    pub fn get_endpoint(&self) -> Url {
        (*self.url.lock().unwrap()).clone()
    }
}

impl WalletNetworkInterface for IndexerRestApiNetworkInterface {
    type Error = IndexerRestApiNetworkInterfaceError;

    async fn query_substate(
        &self,
        substate_id: &SubstateId,
        version: Option<u64>,
        local_search_only: bool,
    ) -> Result<SubstateQueryResult, Self::Error> {
        let client = self.get_client()?;
        let result = client
            .get_substate(substate_id, GetSubstateRequest {
                version,
                local_search_only,
            })
            .await?;
        Ok(SubstateQueryResult {
            version: result.version,
            substate: result.substate,
        })
    }

    async fn get_substates(&self, substate_ids: Vec<SubstateId>) -> Result<HashMap<SubstateId, Substate>, Self::Error> {
        // The indexer's substates/fetch endpoint accepts at most this many IDs per request, so larger
        // requests are split into sequential batches. Substates the indexer cannot find are omitted from
        // the result, not an error.
        const MAX_IDS_PER_REQUEST: usize = 20;

        let client = self.get_client()?;
        let mut substates = HashMap::with_capacity(substate_ids.len());
        for chunk in substate_ids.chunks(MAX_IDS_PER_REQUEST) {
            let resp = client
                .fetch_substates(GetSubstatesRequest {
                    requests: chunk.to_vec().try_into().map_err(|_| {
                        IndexerRestApiNetworkInterfaceError::IndexerClientError(
                            IndexerRestClientError::RequestInvariant {
                                details: "Too many substate IDs requested".to_string(),
                            },
                        )
                    })?,
                    cached_only: false,
                })
                .await?;
            substates.extend(resp.substates);
        }

        Ok(substates)
    }

    async fn submit_transaction(&self, transaction: Transaction) -> Result<TransactionId, Self::Error> {
        let transaction = TransactionEnvelope::encode(transaction)?;
        self.submit_transaction_envelope(transaction).await
    }

    async fn submit_transaction_envelope(
        &self,
        transaction: TransactionEnvelope,
    ) -> Result<TransactionId, Self::Error> {
        let client = self.get_client()?;
        let result = client
            .submit_transaction(SubmitTransactionRequest { transaction })
            .await?;
        Ok(result.transaction_id)
    }

    async fn submit_dry_run_transaction(
        &self,
        transaction: Transaction,
    ) -> Result<TransactionQueryResult, Self::Error> {
        if !transaction.is_dry_run() {
            return Err(IndexerRestApiNetworkInterfaceError::IndexerClientError(
                IndexerRestClientError::RequestFailedWithStatus {
                    code: INVALID_REQUEST_CODE,
                    message: "Transaction must be marked as dry-run".to_string(),
                },
            ));
        }

        let client = self.get_client()?;
        let resp = client
            .submit_transaction_dry_run(SubmitTransactionRequest {
                transaction: TransactionEnvelope::encode(transaction)?,
            })
            .await?;

        Ok(TransactionQueryResult {
            transaction_id: resp.transaction_id,
            // TODO: clean this up
            result: TransactionFinalizedResult::Finalized {
                final_decision: (&resp.result.finalize.result).into(),
                execution_time: resp.result.execution_time,
                execution_result: Some(Box::new(resp.result)),
                finalized_time: now(),
                abort_details: None,
            },
        })
    }

    async fn query_transaction_result(
        &self,
        transaction_id: TransactionId,
    ) -> Result<TransactionQueryResult, Self::Error> {
        let client = self.get_client()?;
        let resp = client
            .get_transaction_result(GetTransactionResultRequest { transaction_id })
            .await?;

        Ok(TransactionQueryResult {
            transaction_id,
            result: convert_indexer_result_to_wallet_result(resp.result),
        })
    }

    async fn subscribe_transaction_finalized(&self) -> Result<TransactionFinalizedStream<Self::Error>, Self::Error> {
        let client = self.get_client()?;
        let events = client.sse_events().await?;
        let stream = events
            .map_err(|e| IndexerRestApiNetworkInterfaceError::StreamDecodeError(e.into()))
            .try_filter_map(|event| async move {
                if event.event_type != IndexerEvent::TRANSACTION_FINALIZED_EVENT_NAME {
                    return Ok(None);
                }
                // A payload this client cannot decode is skipped rather than ending the subscription: the wallet
                // queries a transaction's result after it has stayed silent, so a skipped event costs latency, not
                // correctness.
                let event: TransactionFinalizedEvent = match event.try_parse_event() {
                    Ok(event) => event,
                    Err(e) => {
                        warn!(
                            target: LOG_TARGET,
                            "Skipping undecodable {} event: {e}",
                            IndexerEvent::TRANSACTION_FINALIZED_EVENT_NAME
                        );
                        return Ok(None);
                    },
                };
                Ok(Some(TransactionFinalizedNotification {
                    transaction_id: event.transaction_id,
                    outcome: event.outcome,
                }))
            });
        Ok(stream.boxed())
    }

    async fn fetch_template_definition(
        &self,
        template_address: TemplateAddress,
    ) -> Result<tari_template_abi::TemplateDef, Self::Error> {
        let client = self.get_client()?;
        let resp = client.get_template_definition(template_address).await?;
        Ok(resp.definition)
    }

    async fn stream_stealth_utxo_updates(
        &self,
        from_epoch: Epoch,
        resource_address: ResourceAddress,
        shard_state_versions: Vec<(Shard, StateVersion)>,
        unspent_only: bool,
    ) -> Result<UtxoUpdateStream<Self::Error>, Self::Error> {
        let client = self.get_client()?;
        let stream = client
            .stream_utxo_updates_protobuf(GetUtxoUpdatesRequest {
                from_epoch,
                shard_state_versions,
                resource_address,
                unspent_only,
                per_shard_limit: 1000,
            })
            .await?;
        let stream = stream
            .map_err(|e| IndexerRestApiNetworkInterfaceError::StreamDecodeError(e.into()))
            .and_then(|res| async move {
                let sos = res.sos.map(|sos| StartOfShard {
                    shard: Shard::from(sos.shard),
                    max_state_version: StateVersion::from(sos.max_state_version),
                    has_more: sos.has_more,
                });
                let update = res
                    .update
                    .map(|u| match u {
                        protobuf::WalletUtxoUpdate::Unspent(unspent) => {
                            let public_nonce =
                                RistrettoPublicKeyBytes::from_bytes(&unspent.public_nonce).map_err(|e| {
                                    IndexerRestApiNetworkInterfaceError::StreamDecodeError(anyhow!(
                                        "Failed to decode public nonce: {e}"
                                    ))
                                })?;
                            Ok::<_, IndexerRestApiNetworkInterfaceError>(WalletUtxoUpdate::Unspent(UtxoUnspent {
                                tag: unspent.tag.into(),
                                public_nonce,
                            }))
                        },
                        protobuf::WalletUtxoUpdate::Spent(spent) => {
                            let id_arr = copy_fixed_checked(&spent.id).ok_or_else(|| {
                                IndexerRestApiNetworkInterfaceError::StreamDecodeError(anyhow!(
                                    "Failed to decode UTXO ID, incorrect length"
                                ))
                            })?;
                            Ok::<_, IndexerRestApiNetworkInterfaceError>(WalletUtxoUpdate::Spent(UtxoSpent {
                                id: UtxoId::from_array(id_arr),
                                version: spent.version,
                            }))
                        },
                        protobuf::WalletUtxoUpdate::Burnt(burnt) => {
                            let id_arr = copy_fixed_checked(&burnt.id).ok_or_else(|| {
                                IndexerRestApiNetworkInterfaceError::StreamDecodeError(anyhow!(
                                    "Failed to decode UTXO ID, incorrect length"
                                ))
                            })?;
                            Ok::<_, IndexerRestApiNetworkInterfaceError>(WalletUtxoUpdate::Burnt(UtxoBurnt {
                                id: UtxoId::from_array(id_arr),
                                version: burnt.version,
                            }))
                        },
                    })
                    .transpose()?;
                let eos = res.eos.map(|eos| EndOfShard {
                    max_state_version: eos.max_state_version.into(),
                });

                Ok(UtxoUpdatePayload { sos, update, eos })
            });
        Ok(stream.boxed())
    }

    async fn get_unspent_utxos(
        &self,
        resource_address: ResourceAddress,
        tag_and_nonce_pairs: Vec<(UtxoTag, RistrettoPublicKeyBytes)>,
    ) -> Result<Vec<(UtxoId, Utxo)>, Self::Error> {
        let client = self.get_client()?;
        // TODO: Given the potential size of substates protobuf, json + hex encoding may be too inefficient. Consider
        // supporting the application/x-protobuf content type in the indexer REST API.
        let resp = client
            .get_utxos(GetUtxosRequest {
                resource_address,
                tag_and_nonce_pairs,
            })
            .await?;
        Ok(resp.utxos)
    }

    async fn list_watched_substates(
        &self,
        template_address: Option<TemplateAddress>,
        limit: Option<u64>,
        offset: Option<u64>,
    ) -> Result<Vec<WatchedSubstateItem>, Self::Error> {
        let client = self.get_client()?;

        let resp = client
            .list_watched_substates(ListWatchedSubstatesRequest {
                template_address,
                limit,
                offset,
            })
            .await?;

        Ok(resp.substates)
    }

    async fn get_current_epoch(&self) -> Result<Epoch, Self::Error> {
        let client = self.get_client()?;
        let stats = client.get_epoch_manager_stats().await?;
        Ok(stats.current_epoch)
    }

    async fn wait_until_ready(&self) -> Result<(), Self::Error> {
        let client = self.get_client()?;
        client.wait_until_ready().await?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IndexerRestApiNetworkInterfaceError {
    #[error("Indexer client error: {0}")]
    IndexerClientError(#[from] IndexerRestClientError),
    #[error("Indexer parse error : {0}")]
    IndexerParseError(#[from] ParseError),
    #[error("Stream decode error: {0}")]
    StreamDecodeError(anyhow::Error),
    #[error("Transaction encode error: {source}")]
    EncodeError {
        #[from]
        source: tari_bor::BorError,
    },
}

impl IsNotFoundError for IndexerRestApiNetworkInterfaceError {
    fn is_not_found_error(&self) -> bool {
        match self {
            IndexerRestApiNetworkInterfaceError::IndexerClientError(err) => err.is_not_found_error(),
            _ => false,
        }
    }
}

impl TransactionStatusResponseError for IndexerRestApiNetworkInterfaceError {
    fn get_status(&self) -> ResponseErrorStatus {
        match self {
            IndexerRestApiNetworkInterfaceError::IndexerClientError(err) => {
                if err.is_not_found_error() {
                    return ResponseErrorStatus::NotFound {
                        message: "The requested resource was not found".to_string(),
                    };
                }
                match err {
                    IndexerRestClientError::RequestFailedWithStatus { code, message }
                        if *code == INVALID_REQUEST_CODE =>
                    {
                        ResponseErrorStatus::TransactionRejected {
                            message: message.clone(),
                        }
                    },
                    IndexerRestClientError::RequestFailedWithStatus { code, message } => {
                        ResponseErrorStatus::InternalError {
                            message: format!("Indexer request failed with status {code}: {message}"),
                        }
                    },
                    IndexerRestClientError::ErrorResponse { source, details } => {
                        if source.status().map(|s| s.as_u16()) == Some(INVALID_REQUEST_CODE as u16) {
                            ResponseErrorStatus::TransactionRejected {
                                message: format!("{}. Details: {}", source, details.display()),
                            }
                        } else {
                            ResponseErrorStatus::InternalError {
                                message: format!("Indexer error: {}", source),
                            }
                        }
                    },
                    _ => ResponseErrorStatus::InternalError {
                        message: format!("Indexer client error: {err}"),
                    },
                }
            },
            IndexerRestApiNetworkInterfaceError::IndexerParseError(e) => ResponseErrorStatus::InternalError {
                message: format!("Indexer parse error: {e}"),
            },
            IndexerRestApiNetworkInterfaceError::StreamDecodeError(e) => ResponseErrorStatus::InternalError {
                message: format!("Indexer stream decode error: {e}"),
            },
            IndexerRestApiNetworkInterfaceError::EncodeError { source } => ResponseErrorStatus::InternalError {
                message: format!("Transaction encode error: {source}"),
            },
        }
    }

    fn get_error_message(&self) -> String {
        self.to_string()
    }
}

/// These types are identical, however in order to keep the wallet decoupled from the indexer, we define two types and
/// this conversion function.
// TODO: the common interface and types between the wallet and indexer could be made into a shared "view of the network"
// interface and we can avoid defining two types.
fn convert_indexer_result_to_wallet_result(result: IndexerTransactionFinalizedResult) -> TransactionFinalizedResult {
    match result {
        IndexerTransactionFinalizedResult::Pending => TransactionFinalizedResult::Pending,
        IndexerTransactionFinalizedResult::Finalized {
            final_decision,
            execution_result,
            finalized_time,
            execution_time,
            abort_details,
        } => TransactionFinalizedResult::Finalized {
            final_decision,
            execution_result,
            execution_time,
            finalized_time,
            abort_details,
        },
        IndexerTransactionFinalizedResult::Rejected { details, rejected_time } => {
            TransactionFinalizedResult::Rejected { details, rejected_time }
        },
    }
}

fn now() -> PrimitiveDateTime {
    let now = OffsetDateTime::now_utc();
    PrimitiveDateTime::new(now.date(), now.time())
}
