//! Low-level Notion API client (core protocol layer).
//!
//! Responsibilities: HTTP request wrapping, token injection, rate limiting
//! (429 backoff + retry), and raw JSON deserialization. Domain modules
//! (e.g. `todo`) build their mapping on top, so no module re-implements
//! Notion API logic.
//!
//! Two-layer architecture per [F004](../../docs/adr/F004-shared-notion-client.md):
//! this file is the "low-level shared SDK"; domain modules are the
//! "high-level semantic business layer".
//!
//! Error handling follows the project convention (see [`error`] and
//! [agents.md](../../agents.md)): 401/403 → `AgentError::Auth`, other
//! non-2xx → `AgentError::Network`, body-parse failure → `AgentError::Other`.
//! No new variants that duplicate the existing enum.

use std::time::Duration;

use reqwest::Method;
use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{AgentError, Result};

/// Notion REST API base URL.
const NOTION_API: &str = "https://api.notion.com/v1";
/// Pinned Notion API version (fixed for back-compat).
const NOTION_VERSION: &str = "2022-06-28";

/// Shared Notion client.
///
/// Holds a `reqwest::Client` with `Authorization` and `Notion-Version`
/// headers already injected; all requests reuse the same connection pool.
pub struct NotionClient {
    client: reqwest::Client,
}

impl NotionClient {
    /// Build a client from an Integration Token.
    ///
    /// No `.unwrap()`: header parse / client-build failures are folded
    /// into `AgentError`.
    pub fn new(token: String) -> Result<Self> {
        let mut headers = HeaderMap::new();
        let auth = format!("Bearer {token}");
        headers.insert(
            reqwest::header::AUTHORIZATION,
            HeaderValue::from_str(&auth)
                .map_err(|e| AgentError::Auth(format!("invalid token (header): {e}")))?,
        );
        headers.insert("Notion-Version", HeaderValue::from_static(NOTION_VERSION));

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(30))
            .user_agent(format!("everyday/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| AgentError::Network(format!("build notion client: {e}")))?;
        Ok(Self { client })
    }

    /// Generic request: send and deserialize the response into `R`.
    ///
    /// Auto-handles 429 rate limiting: reads the `Retry-After` header
    /// (defaults to 1s when absent), backs off, then retries **once** so
    /// Agent batch ops (e.g. updating task statuses one by one) don't break
    /// the stream. Notion allows ~3 req/s; one retry smooths transient limits.
    pub async fn request<B, R>(&self, method: Method, path: &str, body: Option<&B>) -> Result<R>
    where
        B: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        let url = format!("{NOTION_API}{path}");
        // Only retry once to avoid infinite backoff.
        for attempt in 0..2 {
            let mut req = self.client.request(method.clone(), &url);
            if let Some(b) = body {
                req = req.json(b);
            }
            let resp = req
                .send()
                .await
                .map_err(|e| AgentError::Network(format!("notion request failed: {e}")))?;
            let status = resp.status();

            // 429: back off and retry once, only on the first attempt.
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS && attempt == 0 {
                let wait = resp
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(1);
                tokio::time::sleep(Duration::from_secs(wait)).await;
                continue;
            }

            let text = resp
                .text()
                .await
                .map_err(|e| AgentError::Network(format!("read response body: {e}")))?;
            if !status.is_success() {
                let msg = extract_message(&text);
                if status == reqwest::StatusCode::UNAUTHORIZED
                    || status == reqwest::StatusCode::FORBIDDEN
                {
                    return Err(AgentError::Auth(format!(
                        "Notion API auth failed ({}): {}",
                        status, msg
                    )));
                }
                return Err(AgentError::Network(format!(
                    "Notion API error ({}): {}",
                    status, msg
                )));
            }
            let value: R = serde_json::from_str(&text)
                .map_err(|e| AgentError::Other(format!("parse notion response: {e}")))?;
            return Ok(value);
        }
        // Unreachable in practice: the loop only continues on attempt==0
        // with a 429, so the second pass must return.
        Err(AgentError::Network(
            "notion request exhausted retries".into(),
        ))
    }

    /// GET request (no body).
    pub async fn get<R: DeserializeOwned>(&self, path: &str) -> Result<R> {
        self.request(Method::GET, path, None::<&()>).await
    }

    /// POST request (JSON body).
    pub async fn post<B: Serialize + ?Sized, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<R> {
        self.request(Method::POST, path, Some(body)).await
    }

    /// PATCH request (JSON body).
    pub async fn patch<B: Serialize + ?Sized, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<R> {
        self.request(Method::PATCH, path, Some(body)).await
    }
}

/// Extract the `message` field from a Notion error body; fall back to
/// the raw text if extraction fails.
fn extract_message(text: &str) -> String {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| {
            v.get("message")
                .and_then(|m| m.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_client_with_token() {
        // Construction must not unwrap/panic.
        let _c = NotionClient::new("ntn_test".into()).unwrap();
    }

    #[test]
    fn extract_message_prefers_field() {
        assert_eq!(extract_message(r#"{"message":"boom","code":"x"}"#), "boom");
    }

    #[test]
    fn extract_message_falls_back_to_raw() {
        assert_eq!(extract_message("not json"), "not json");
    }
}
