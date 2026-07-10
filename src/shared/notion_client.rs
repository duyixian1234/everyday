//! 底层 Notion API 客户端（核心协议层）。
//!
//! 职责：HTTP 请求封装、Token 注入、速率限制（429 退避重试）、
//! 原始 JSON 反序列化。业务模块（如 `todo`）在其上做领域映射，
//! 避免每个模块重复编写 Notion API 逻辑。
//!
//! 设计参考待办模块设计文档的两层架构：本文件为「底层共享 SDK」，
//! 业务模块为「上层语义化业务」。
//!
//! 错误处理遵循项目既有约定（见 `error.rs` 与 `agents.md`）：
//! 401/403 映射为 `AgentError::Auth`，其它非 2xx 映射为 `AgentError::Network`，
//! 响应体解析失败映射为 `AgentError::Other`。不新增与现有枚举重复的变体。

use std::time::Duration;

use reqwest::Method;
use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{AgentError, Result};

/// Notion REST API 基址。
const NOTION_API: &str = "https://api.notion.com/v1";
/// 使用的 Notion API 版本（固定，向后兼容）。
const NOTION_VERSION: &str = "2022-06-28";

/// 共享 Notion 客户端。
///
/// 持有一个已注入 `Authorization` 与 `Notion-Version` 头的
/// `reqwest::Client`，所有请求复用同一连接池。
pub struct NotionClient {
    client: reqwest::Client,
}

impl NotionClient {
    /// 用 Integration Token 构造客户端。
    ///
    /// 不使用 `.unwrap()`：头解析/客户端构建失败都收拢为 `AgentError`。
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

    /// 通用请求：发送并解析响应为 `R`。
    ///
    /// 自动处理 429 频率限制：读取响应头 `Retry-After`（不存在则默认 1 秒）
    /// 退避后**单次重试**，确保 Agent 批量操作（如逐个更新任务状态）不断流。
    /// Notion 限制约每秒 3 次请求，单次重试足以平滑偶发限流。
    pub async fn request<B, R>(&self, method: Method, path: &str, body: Option<&B>) -> Result<R>
    where
        B: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        let url = format!("{NOTION_API}{path}");
        // 仅重试一次，避免无限退避。
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

            // 429：仅在首次尝试时退避重试一次。
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
        // 理论不可达：循环仅可能在 attempt==0 且 429 时 continue，第二次必返回。
        Err(AgentError::Network(
            "notion request exhausted retries".into(),
        ))
    }

    /// GET 请求（无请求体）。
    pub async fn get<R: DeserializeOwned>(&self, path: &str) -> Result<R> {
        self.request(Method::GET, path, None::<&()>).await
    }

    /// POST 请求（带 JSON 请求体）。
    pub async fn post<B: Serialize + ?Sized, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<R> {
        self.request(Method::POST, path, Some(body)).await
    }

    /// PATCH 请求（带 JSON 请求体）。
    pub async fn patch<B: Serialize + ?Sized, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<R> {
        self.request(Method::PATCH, path, Some(body)).await
    }
}

/// 从 Notion 错误响应体提取 `message` 字段；提取失败则回退为原始文本。
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
        // 构造不应 unwrap/panic。
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
