use std::sync::Arc;

use futures::{StreamExt, stream::BoxStream};
use http::header::WWW_AUTHENTICATE;
use reqwest::header::ACCEPT;
use rmcp::{
    model::{ClientJsonRpcMessage, ServerJsonRpcMessage},
    transport::streamable_http_client::{
        AuthRequiredError, SseError, StreamableHttpClient, StreamableHttpError,
        StreamableHttpPostResponse,
    },
};
use sse_stream::{Sse, SseStream};
use tracing::debug;

const HEADER_SESSION_ID: &str = "Mcp-Session-Id";
const EVENT_STREAM_MIME_TYPE: &str = "text/event-stream";
const JSON_MIME_TYPE: &str = "application/json";

/// A wrapper around `reqwest::Client` that tolerates missing Content-Type
/// headers on POST responses. Some MCP servers (e.g., z.ai) return 200 OK
/// with no Content-Type for the `initialized` notification acknowledgment.
/// Standard rmcp rejects these with `UnexpectedContentType(None)`.
///
/// This wrapper treats missing/unexpected Content-Type on 200 OK as `Accepted`,
/// matching the Go gateway's lenient behavior.
#[derive(Clone)]
pub struct LenientClient {
    inner: reqwest::Client,
}

impl LenientClient {
    pub fn new(client: reqwest::Client) -> Self {
        Self { inner: client }
    }
}

impl StreamableHttpClient for LenientClient {
    type Error = reqwest::Error;

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        last_event_id: Option<String>,
        auth_token: Option<String>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        // Delegate directly — get_stream Content-Type strictness is fine
        self.inner
            .get_stream(uri, session_id, last_event_id, auth_token)
            .await
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_token: Option<String>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        // Delegate directly
        self.inner.delete_session(uri, session_id, auth_token).await
    }

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_token: Option<String>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        let mut request = self
            .inner
            .post(uri.as_ref())
            .header(ACCEPT, [EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE].join(", "));
        if let Some(auth_header) = auth_token {
            request = request.bearer_auth(auth_header);
        }
        if let Some(session_id) = session_id {
            request = request.header(HEADER_SESSION_ID, session_id.as_ref());
        }
        let response = request.json(&message).send().await?;

        // Handle 401 Unauthorized
        if response.status() == reqwest::StatusCode::UNAUTHORIZED
            && let Some(header) = response.headers().get(WWW_AUTHENTICATE)
        {
            let header = header
                .to_str()
                .map_err(|_| {
                    StreamableHttpError::UnexpectedServerResponse(
                        "invalid www-authenticate header value".into(),
                    )
                })?
                .to_string();
            return Err(StreamableHttpError::AuthRequired(AuthRequiredError {
                www_authenticate_header: header,
            }));
        }

        // 202 Accepted / 204 No Content → Accepted
        let status = response.status();
        if matches!(
            status,
            reqwest::StatusCode::ACCEPTED | reqwest::StatusCode::NO_CONTENT
        ) {
            return Ok(StreamableHttpPostResponse::Accepted);
        }

        let content_type = response.headers().get(reqwest::header::CONTENT_TYPE);
        let session_id_header = response.headers().get(HEADER_SESSION_ID);
        let session_id_val = session_id_header
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        match content_type {
            Some(ct) if ct.as_bytes().starts_with(EVENT_STREAM_MIME_TYPE.as_bytes()) => {
                let event_stream = SseStream::from_byte_stream(response.bytes_stream()).boxed();
                Ok(StreamableHttpPostResponse::Sse(
                    event_stream,
                    session_id_val,
                ))
            }
            Some(ct) if ct.as_bytes().starts_with(JSON_MIME_TYPE.as_bytes()) => {
                let message: ServerJsonRpcMessage = response.json().await?;
                Ok(StreamableHttpPostResponse::Json(message, session_id_val))
            }
            _ => {
                // LENIENT: instead of returning UnexpectedContentType error,
                // try to parse as JSON first, then fall back to Accepted.
                // This handles servers that return 200 OK with no Content-Type
                // for notification acknowledgments (e.g., z.ai backends).
                debug!(
                    content_type = ?content_type.map(|ct| String::from_utf8_lossy(ct.as_bytes()).to_string()),
                    status = %status,
                    "missing or unexpected Content-Type, treating as Accepted"
                );

                // Try JSON parse — some servers send JSON without the header
                let bytes = response.bytes().await?;
                if !bytes.is_empty()
                    && let Ok(message) = serde_json::from_slice::<ServerJsonRpcMessage>(&bytes)
                {
                    return Ok(StreamableHttpPostResponse::Json(message, session_id_val));
                }

                // Empty body or unparseable → treat as Accepted
                Ok(StreamableHttpPostResponse::Accepted)
            }
        }
    }
}
