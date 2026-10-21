//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{sync::Arc, time::Duration};

use reqwest::{IntoUrl, Url, header, header::HeaderMap};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tari_engine_types::substate::SubstateId;
use tari_ootle_transaction::TransactionId;
use tari_template_lib_types::{ResourceAddress, TemplateAddress, TransactionReceiptAddress};
use tokio::sync::watch;

use crate::{
    error::IndexerRestClientError,
    protobuf,
    protobuf_stream::ProtobufStream,
    sse::{SseEventStream, SseEventStreamBuilder},
    types::{
        GetCommitteeRequest,
        GetCommitteeResponse,
        GetConnectionsResponse,
        GetEpochManagerStatsResponse,
        GetLatestEpochCheckpointRequest,
        GetLatestEpochCheckpointResponse,
        GetNetworkEconomicsResponse,
        GetNetworkInfoResponse,
        GetNetworkSyncStateResponse,
        GetResourceResponse,
        GetSubstateRequest,
        GetSubstateResponse,
        GetSubstatesRequest,
        GetSubstatesResponse,
        GetTemplateDefinitionResponse,
        GetTransactionReceiptResponse,
        GetTransactionResponse,
        GetTransactionResultQuery,
        GetTransactionResultRequest,
        GetTransactionResultResponse,
        GetUtxoUpdatesRequest,
        GetUtxosRequest,
        GetUtxosResponse,
        IndexerReadyResponse,
        ListEpochCheckpointsRequest,
        ListEpochCheckpointsResponse,
        ListRecentTransactionsRequest,
        ListRecentTransactionsResponse,
        ListTemplateCatalogueRequest,
        ListTemplateCatalogueResponse,
        ListTransactionReceiptsRequest,
        ListTransactionReceiptsResponse,
        ListUtxosRequest,
        ListUtxosResponse,
        ListWatchedSubstatesRequest,
        ListWatchedSubstatesResponse,
        ListWatchedTemplatesResponse,
        QueryTransactionEventsRequest,
        QueryTransactionEventsResponse,
        StreamTransactionEventsRequest,
        SubmitTransactionDryRunResponse,
        SubmitTransactionRequest,
        SubmitTransactionResponse,
        TemplateCatalogueItem,
    },
};

/// A REST client for the Ootle indexer.
///
/// Clones share one endpoint: [`set_endpoint`](Self::set_endpoint) on any clone redirects every clone's
/// subsequent requests to the new indexer.
#[derive(Debug, Clone)]
pub struct IndexerRestApiClient {
    client: reqwest::Client,
    endpoint: Arc<watch::Sender<Url>>,
}

impl IndexerRestApiClient {
    pub fn connect<T: IntoUrl>(endpoint: T) -> Result<Self, IndexerRestClientError> {
        Self::connect_internal(endpoint, None, None)
    }

    pub fn connect_with_timeout<T: IntoUrl>(endpoint: T, timeout: Duration) -> Result<Self, IndexerRestClientError> {
        Self::connect_internal(endpoint, Some(timeout), None)
    }

    /// Connects with a bound on establishing each TCP connection only. Unlike
    /// [`connect_with_timeout`](Self::connect_with_timeout), responses may take as long as they need, so SSE
    /// subscriptions and long polls stay usable.
    pub fn connect_with_connect_timeout<T: IntoUrl>(
        endpoint: T,
        connect_timeout: Duration,
    ) -> Result<Self, IndexerRestClientError> {
        Self::connect_internal(endpoint, None, Some(connect_timeout))
    }

    fn connect_internal(
        endpoint: impl IntoUrl,
        timeout: Option<Duration>,
        connect_timeout: Option<Duration>,
    ) -> Result<Self, IndexerRestClientError> {
        let client_builder = reqwest::Client::builder().default_headers({
            let mut headers = HeaderMap::with_capacity(1);
            headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
            headers
        });

        let client_builder = if let Some(timeout) = timeout {
            client_builder.timeout(timeout)
        } else {
            client_builder
        };
        let client_builder = if let Some(connect_timeout) = connect_timeout {
            client_builder.connect_timeout(connect_timeout)
        } else {
            client_builder
        };

        let client = client_builder.build()?;

        let (endpoint, _) = watch::channel(endpoint.into_url()?);
        Ok(Self {
            client,
            endpoint: Arc::new(endpoint),
        })
    }

    /// The indexer URL that requests are currently sent to.
    pub fn endpoint(&self) -> Url {
        self.endpoint.borrow().clone()
    }

    /// Sends all subsequent requests, from this client and every clone of it, to `endpoint`.
    ///
    /// Requests and streams already in flight stay on the previous indexer. Subscribers from
    /// [`subscribe_endpoint_changes`](Self::subscribe_endpoint_changes) are notified only when the URL actually
    /// changes.
    pub fn set_endpoint<T: IntoUrl>(&self, endpoint: T) -> Result<(), IndexerRestClientError> {
        let endpoint = endpoint.into_url()?;
        self.endpoint.send_if_modified(|current| {
            if *current == endpoint {
                return false;
            }
            *current = endpoint;
            true
        });
        Ok(())
    }

    /// Returns a handle that resolves each time [`set_endpoint`](Self::set_endpoint) moves this client to a
    /// different indexer, so that long-lived streams can reconnect to it.
    pub fn subscribe_endpoint_changes(&self) -> EndpointChanges {
        EndpointChanges {
            rx: self.endpoint.subscribe(),
        }
    }

    pub async fn get_connections(&self) -> Result<GetConnectionsResponse, IndexerRestClientError> {
        self.send_get("network/connections", ()).await
    }

    pub async fn get_network_economics(&self) -> Result<GetNetworkEconomicsResponse, IndexerRestClientError> {
        self.send_get("network/economics", ()).await
    }

    pub async fn get_substate(
        &self,
        id: &SubstateId,
        req: GetSubstateRequest,
    ) -> Result<GetSubstateResponse, IndexerRestClientError> {
        self.send_get(format!("substates/{id}"), req).await
    }

    pub async fn fetch_substates(
        &self,
        req: GetSubstatesRequest,
    ) -> Result<GetSubstatesResponse, IndexerRestClientError> {
        self.send_post("substates/fetch", req).await
    }

    pub async fn submit_transaction(
        &self,
        req: SubmitTransactionRequest,
    ) -> Result<SubmitTransactionResponse, IndexerRestClientError> {
        self.send_post("transactions", req).await
    }

    pub async fn submit_transaction_dry_run(
        &self,
        req: SubmitTransactionRequest,
    ) -> Result<SubmitTransactionDryRunResponse, IndexerRestClientError> {
        self.send_post("transactions/dry-run", req).await
    }

    pub async fn get_transaction(
        &self,
        transaction_id: TransactionId,
    ) -> Result<GetTransactionResponse, IndexerRestClientError> {
        self.send_get(format!("transactions/{transaction_id}"), ()).await
    }

    pub async fn get_transaction_result(
        &self,
        req: GetTransactionResultRequest,
    ) -> Result<GetTransactionResultResponse, IndexerRestClientError> {
        self.send_get(
            format!("transactions/{}/result", req.transaction_id),
            GetTransactionResultQuery {
                include_proof: req.include_proof,
            },
        )
        .await
    }

    pub async fn list_recent_transactions(
        &self,
        req: ListRecentTransactionsRequest,
    ) -> Result<ListRecentTransactionsResponse, IndexerRestClientError> {
        self.send_get("transactions/recent", req).await
    }

    pub async fn get_template_definition(
        &self,
        template_address: TemplateAddress,
    ) -> Result<GetTemplateDefinitionResponse, IndexerRestClientError> {
        self.send_get(format!("templates/{template_address}"), ()).await
    }

    pub async fn list_template_catalogue(
        &self,
        req: ListTemplateCatalogueRequest,
    ) -> Result<ListTemplateCatalogueResponse, IndexerRestClientError> {
        self.send_get("templates/catalogue", req).await
    }

    pub async fn get_template_catalogue_entry(
        &self,
        template_address: TemplateAddress,
    ) -> Result<TemplateCatalogueItem, IndexerRestClientError> {
        self.send_get(format!("templates/catalogue/{template_address}"), ())
            .await
    }

    pub async fn list_watched_templates(&self) -> Result<ListWatchedTemplatesResponse, IndexerRestClientError> {
        self.send_get("templates/watched", ()).await
    }

    pub async fn list_watched_substates(
        &self,
        req: ListWatchedSubstatesRequest,
    ) -> Result<ListWatchedSubstatesResponse, IndexerRestClientError> {
        self.send_get("substates/watched", req).await
    }

    pub async fn query_transaction_events(
        &self,
        req: QueryTransactionEventsRequest,
    ) -> Result<QueryTransactionEventsResponse, IndexerRestClientError> {
        self.send_get("transactions/events", req).await
    }

    pub async fn get_epoch_manager_stats(&self) -> Result<GetEpochManagerStatsResponse, IndexerRestClientError> {
        self.send_get("epoch-manager/stats", ()).await
    }

    pub async fn stream_utxo_updates_protobuf(
        &self,
        req: GetUtxoUpdatesRequest,
    ) -> Result<ProtobufStream<protobuf::UtxoUpdatePayload>, IndexerRestClientError> {
        const PATH: &str = "utxos/stream";
        let url = self.url_for(PATH);

        let resp = self
            .client
            .post(&url)
            .header(header::ACCEPT, "application/x-protobuf")
            .json(&req)
            .send()
            .await?;
        let resp = require_success_status(resp)?;

        let stream = resp.bytes_stream();
        let stream = ProtobufStream::<protobuf::UtxoUpdatePayload>::new(stream);
        Ok(stream)
    }

    pub async fn get_utxos(&self, req: GetUtxosRequest) -> Result<GetUtxosResponse, IndexerRestClientError> {
        self.send_post("utxos/fetch", req).await
    }

    pub async fn list_utxos(&self, req: ListUtxosRequest) -> Result<ListUtxosResponse, IndexerRestClientError> {
        self.send_get("utxos", req).await
    }

    pub async fn list_transaction_receipts(
        &self,
        req: ListTransactionReceiptsRequest,
    ) -> Result<ListTransactionReceiptsResponse, IndexerRestClientError> {
        self.send_get("transaction-receipts", req).await
    }

    pub async fn get_transaction_receipt(
        &self,
        address: TransactionReceiptAddress,
    ) -> Result<GetTransactionReceiptResponse, IndexerRestClientError> {
        // We use as_object_key to get the string representation without the "txreceipt_" prefix
        self.send_get(format!("transaction-receipts/{}", address.as_object_key()), ())
            .await
    }

    pub async fn get_network_info(&self) -> Result<GetNetworkInfoResponse, IndexerRestClientError> {
        self.send_get("network", ()).await
    }

    pub async fn get_network_sync_state(&self) -> Result<GetNetworkSyncStateResponse, IndexerRestClientError> {
        self.send_get("network/stats", ()).await
    }

    pub async fn wait_until_ready(&self) -> Result<IndexerReadyResponse, IndexerRestClientError> {
        self.send_get("wait-until-ready", ()).await
    }

    pub async fn get_resource(&self, addr: ResourceAddress) -> Result<GetResourceResponse, IndexerRestClientError> {
        self.send_get(format!("resources/{addr}"), ()).await
    }

    pub async fn get_tari_resource(&self) -> Result<GetResourceResponse, IndexerRestClientError> {
        self.send_get("resources/tari", ()).await
    }

    pub async fn list_epoch_checkpoints(
        &self,
        req: ListEpochCheckpointsRequest,
    ) -> Result<ListEpochCheckpointsResponse, IndexerRestClientError> {
        self.send_get("epoch-checkpoints", req).await
    }

    pub async fn get_committee(
        &self,
        req: GetCommitteeRequest,
    ) -> Result<GetCommitteeResponse, IndexerRestClientError> {
        self.send_get("committees", req).await
    }

    pub async fn get_latest_epoch_checkpoint(
        &self,
        req: GetLatestEpochCheckpointRequest,
    ) -> Result<GetLatestEpochCheckpointResponse, IndexerRestClientError> {
        self.send_get("epoch-checkpoints/latest", req).await
    }

    pub async fn sse_events(&self) -> Result<SseEventStream, IndexerRestClientError> {
        let sse = self.send_sse("events", ()).await?;
        sse.into_stream()
    }

    pub async fn sse_transaction_events(
        &self,
        req: StreamTransactionEventsRequest,
    ) -> Result<SseEventStream, IndexerRestClientError> {
        let sse = self.send_sse("transactions/events/stream", req).await?;
        sse.into_stream()
    }

    /// Joins `path` onto the current endpoint. The endpoint is read once, so a request built from the result goes
    /// to a single indexer even if [`set_endpoint`](Self::set_endpoint) runs while it is in flight.
    fn url_for(&self, path: &str) -> String {
        format!("{}{}", *self.endpoint.borrow(), path)
    }

    async fn send_sse<P: Into<String>, T: Serialize>(
        &self,
        path: P,
        params: T,
    ) -> Result<SseEventStreamBuilder, IndexerRestClientError> {
        let path = path.into();

        // encode query params
        let query = serde_urlencoded::to_string(&params).map_err(|e| IndexerRestClientError::SerializeRequest {
            path: path.clone(),
            source: e.into(),
        })?;

        let mut url = self.url_for(&path);
        if !query.is_empty() {
            url.push('?');
            url.push_str(&query);
        }

        let resp = self.client.get(url).send().await?;
        Ok(resp.into())
    }

    async fn send_get<P: Into<String>, T: Serialize, R: DeserializeOwned>(
        &self,
        path: P,
        params: T,
    ) -> Result<R, IndexerRestClientError> {
        let path = path.into();

        // encode query params
        let query = serde_urlencoded::to_string(&params).map_err(|e| IndexerRestClientError::SerializeRequest {
            path: path.clone(),
            source: e.into(),
        })?;

        let mut url = self.url_for(&path);
        if !query.is_empty() {
            url.push('?');
            url.push_str(&query);
        }

        let resp = self.client.get(url).send().await?;
        handle_json_response(resp, path).await
    }

    async fn send_post<P: Into<String>, T: Serialize, R: DeserializeOwned>(
        &self,
        path: P,
        request: T,
    ) -> Result<R, IndexerRestClientError> {
        let path = path.into();

        let url = self.url_for(&path);
        let resp = self.client.post(url).json(&request).send().await?;

        handle_json_response(resp, path).await
    }
}

/// Notifies of endpoint changes on an [`IndexerRestApiClient`]. Created by
/// [`IndexerRestApiClient::subscribe_endpoint_changes`].
#[derive(Debug, Clone)]
pub struct EndpointChanges {
    rx: watch::Receiver<Url>,
}

impl EndpointChanges {
    /// Waits until the client's endpoint changes and returns the new URL. Changes made before this handle was
    /// created are not reported.
    ///
    /// Once every clone of the client has been dropped the endpoint can no longer change, and this never resolves.
    pub async fn changed(&mut self) -> Url {
        if self.rx.changed().await.is_err() {
            return std::future::pending().await;
        }
        self.rx.borrow_and_update().clone()
    }
}

/// Passes `resp` through only if its status is 2xx, the only statuses whose body is the requested resource. A 4xx or
/// 5xx becomes [`IndexerRestClientError::ErrorResponse`]; any other status (1xx, or a 3xx that was not followed as a
/// redirect) becomes [`IndexerRestClientError::UnexpectedStatus`].
pub(crate) fn require_success_status(resp: reqwest::Response) -> Result<reqwest::Response, IndexerRestClientError> {
    let resp = resp
        .error_for_status()
        .map_err(|source| IndexerRestClientError::ErrorResponse { source, details: None })?;
    let status = resp.status();
    if !status.is_success() {
        return Err(IndexerRestClientError::UnexpectedStatus {
            status,
            url: resp.url().clone(),
        });
    }
    Ok(resp)
}

async fn handle_json_response<T: DeserializeOwned>(
    resp: reqwest::Response,
    path: String,
) -> Result<T, IndexerRestClientError> {
    if let Some(err) = resp.error_for_status_ref().err() {
        return Err(IndexerRestClientError::ErrorResponse {
            source: err,
            details: read_error_details(resp).await,
        });
    }
    let resp = require_success_status(resp)?;
    match resp.json().await {
        Ok(r) => Ok(r),
        Err(e) => Err(IndexerRestClientError::DeserializeResponse { path, source: e.into() }),
    }
}

/// The most of an error response body the client reads. An indexer error body is `{"error": "<message>"}`, so a
/// longer body carries nothing the client uses.
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
struct ErrorBody {
    error: Option<String>,
}

/// Returns the `error` message of an error response, or `None` if the body is longer than
/// [`MAX_ERROR_BODY_BYTES`] or is not an object with a string `error` field.
async fn read_error_details(mut resp: reqwest::Response) -> Option<String> {
    if resp
        .content_length()
        .is_some_and(|len| len > MAX_ERROR_BODY_BYTES as u64)
    {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.ok()? {
        if body.len() + chunk.len() > MAX_ERROR_BODY_BYTES {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice::<ErrorBody>(&body).ok()?.error
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        time::Duration,
    };

    use tari_ootle_common_types::optional::IsNotFoundError;

    use super::*;

    /// Answers one request with `head` followed by `body`, then holds the connection open until the client closes
    /// it. Returns the URL to connect to.
    fn serve_once(head: String, body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut req = [0u8; 4096];
            let mut received = Vec::new();
            while !received.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = sock.read(&mut req).unwrap();
                assert!(n > 0, "the client closed before sending a request");
                received.extend_from_slice(&req[..n]);
            }
            if sock.write_all(head.as_bytes()).is_err() || sock.write_all(&body).is_err() {
                return;
            }
            while matches!(sock.read(&mut req), Ok(n) if n > 0) {}
        });
        format!("http://{addr}")
    }

    /// An error body that opens a JSON object with an `error` message followed by an array that runs past the read
    /// bound.
    fn oversized_error_body() -> Vec<u8> {
        let mut body = b"{\"error\":\"indexer unavailable\",\"data\":[".to_vec();
        body.extend(std::iter::repeat_n(b"1,".as_slice(), 512 * 1024).flatten());
        body
    }

    async fn expect_error_details(url: String) -> Option<String> {
        let client = IndexerRestApiClient::connect(url).unwrap();
        let err = tokio::time::timeout(Duration::from_secs(10), client.get_network_info())
            .await
            .expect("the client kept reading an error body past its bound")
            .unwrap_err();
        match err {
            IndexerRestClientError::ErrorResponse { details, .. } => details,
            other => panic!("expected ErrorResponse, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_error_body_declared_longer_than_the_bound_is_not_read() {
        let url = serve_once(
            "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: \
             1073741824\r\n\r\n"
                .to_string(),
            oversized_error_body(),
        );
        assert_eq!(expect_error_details(url).await, None);
    }

    #[tokio::test]
    async fn an_undelimited_error_body_is_read_only_up_to_the_bound() {
        let url = serve_once(
            "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n"
                .to_string(),
            oversized_error_body(),
        );
        assert_eq!(expect_error_details(url).await, None);
    }

    #[tokio::test]
    async fn an_error_body_yields_its_error_message() {
        let body = br#"{"error":"Substate not found","data":[1,2,3]}"#.to_vec();
        let head = format!(
            "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let client = IndexerRestApiClient::connect(serve_once(head, body)).unwrap();

        let err = client.get_network_info().await.unwrap_err();

        assert!(err.is_not_found_error());
        match err {
            IndexerRestClientError::ErrorResponse { details, .. } => {
                assert_eq!(details.as_deref(), Some("Substate not found"));
            },
            other => panic!("expected ErrorResponse, got {other:?}"),
        }
    }

    #[test]
    fn set_endpoint_redirects_every_clone() {
        let client = IndexerRestApiClient::connect("http://indexer-a.example:18300").unwrap();
        let clone = client.clone();

        clone.set_endpoint("http://indexer-b.example:18300").unwrap();

        assert_eq!(client.endpoint().as_str(), "http://indexer-b.example:18300/");
        assert_eq!(
            client.url_for("substates/fetch"),
            "http://indexer-b.example:18300/substates/fetch"
        );
    }

    #[tokio::test]
    async fn subscribers_hear_a_change_to_a_different_endpoint() {
        let client = IndexerRestApiClient::connect("http://indexer-a.example:18300").unwrap();
        let mut changes = client.subscribe_endpoint_changes();

        client.set_endpoint("http://indexer-b.example:18300").unwrap();

        let url = tokio::time::timeout(Duration::from_secs(1), changes.changed())
            .await
            .expect("change was not reported");
        assert_eq!(url.as_str(), "http://indexer-b.example:18300/");
    }

    #[tokio::test]
    async fn setting_the_current_endpoint_is_not_a_change() {
        let client = IndexerRestApiClient::connect("http://indexer-a.example:18300").unwrap();
        let mut changes = client.subscribe_endpoint_changes();

        client.set_endpoint("http://indexer-a.example:18300/").unwrap();

        tokio::time::timeout(Duration::from_millis(50), changes.changed())
            .await
            .unwrap_err();
    }

    #[test]
    fn an_unparseable_endpoint_is_refused_and_the_current_one_kept() {
        let client = IndexerRestApiClient::connect("http://indexer-a.example:18300").unwrap();

        client.set_endpoint("not a url").unwrap_err();

        assert_eq!(client.endpoint().as_str(), "http://indexer-a.example:18300/");
    }
}
