//! Thin WebDAV client for device sync (ADR D001–D003).
//!
//! Only four methods are needed (RFC 4918): MKCOL (ensure_dir), PROPFIND
//! (list), PUT, GET. Built directly on `reqwest` (already a dependency) with
//! HTTP Basic auth — no dedicated webdav crate (D001 Alternatives).
//!
//! The `WebdavClient` trait is the seam for tests: engine logic runs against
//! `MockWebdavClient` (in-memory), the real network path is `ReqwestClient`.
//! Network errors map to `AgentError::Network`, 401/403 to `Auth`, and the
//! request timeout is 10s (D003).

use async_trait::async_trait;

use crate::error::{AgentError, Result};

/// One remote file's metadata, from a PROPFIND or PUT response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteEntry {
    /// File basename (e.g. `memory.db`).
    pub name: String,
    /// Server ETag (opaque content fingerprint; weak for some servers).
    pub etag: Option<String>,
    /// Server Last-Modified (RFC 7231 HTTP-date), if present.
    pub last_modified: Option<String>,
}

/// The network seam for sync (R018-style fake lives next to the trait).
#[async_trait]
pub trait WebdavClient: Send + Sync {
    /// Ensure the remote directory exists (MKCOL; existing dir is a no-op).
    async fn ensure_dir(&self, dir_url: &str) -> Result<()>;
    /// List the directory (PROPFIND depth 1): name + etag + last-modified.
    async fn list(&self, dir_url: &str) -> Result<Vec<RemoteEntry>>;
    /// Upload `body` as `dir_url/{name}` (PUT), returning server metadata.
    async fn put(&self, dir_url: &str, name: &str, body: &[u8]) -> Result<RemoteEntry>;
    /// Download `dir_url/{name}` (GET), returning the raw bytes.
    async fn get(&self, dir_url: &str, name: &str) -> Result<Vec<u8>>;
}

/// Real implementation: reqwest + HTTP Basic auth, 10s timeout.
pub struct ReqwestClient {
    client: reqwest::Client,
    username: String,
    password: String,
}

impl ReqwestClient {
    /// Build a client from WebDAV credentials. The secret is the application
    /// password from the keyring (never the config file).
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| AgentError::Network(format!("build http client: {e}")))?;
        Ok(Self {
            client,
            username: username.into(),
            password: password.into(),
        })
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.basic_auth(&self.username, Some(&self.password))
    }
}

/// The four WebDAV methods are non-standard HTTP verbs; `http::Method` only
/// has constants for the standard nine, so build them from bytes. The verbs
/// here are all hard-coded, but the constructor is fallible by contract.
fn method(verb: &str) -> Result<reqwest::Method> {
    reqwest::Method::from_bytes(verb.as_bytes())
        .map_err(|e| AgentError::Other(format!("invalid HTTP method {verb:?}: {e}")))
}

#[async_trait]
impl WebdavClient for ReqwestClient {
    async fn ensure_dir(&self, dir_url: &str) -> Result<()> {
        let resp = self
            .auth(self.client.request(method("MKCOL")?, dir_url))
            .send()
            .await
            .map_err(network_err)?;
        // 201 created; 200/204 already existed; 405 method not allowed /
        // 409 conflict mean the collection already exists on most servers.
        if matches!(resp.status().as_u16(), 200 | 201 | 204 | 301 | 405 | 409) {
            return Ok(());
        }
        auth_or_err(resp, "create directory").await
    }

    async fn list(&self, dir_url: &str) -> Result<Vec<RemoteEntry>> {
        let resp = self
            .auth(
                self.client
                    .request(method("PROPFIND")?, dir_url)
                    .header("Depth", "1"),
            )
            .send()
            .await
            .map_err(network_err)?;
        if resp.status().as_u16() == 207 {
            let body = resp
                .bytes()
                .await
                .map_err(|e| AgentError::Network(format!("read PROPFIND body: {e}")))?;
            return parse_multistatus(&body);
        }
        auth_or_err(resp, "list directory").await
    }

    async fn put(&self, dir_url: &str, name: &str, body: &[u8]) -> Result<RemoteEntry> {
        let url = join_url(dir_url, name);
        let resp = self
            .auth(self.client.put(url).body(body.to_vec()))
            .send()
            .await
            .map_err(network_err)?;
        let status = resp.status().as_u16();
        if matches!(status, 200 | 201 | 204) {
            let etag = resp
                .headers()
                .get("etag")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let last_modified = resp
                .headers()
                .get("last-modified")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            return Ok(RemoteEntry {
                name: name.to_string(),
                etag,
                last_modified,
            });
        }
        auth_or_err(resp, format!("upload {name}")).await
    }

    async fn get(&self, dir_url: &str, name: &str) -> Result<Vec<u8>> {
        let url = join_url(dir_url, name);
        let resp = self
            .auth(self.client.get(url))
            .send()
            .await
            .map_err(network_err)?;
        if resp.status().is_success() {
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| AgentError::Network(format!("read download body: {e}")))?;
            return Ok(bytes.to_vec());
        }
        auth_or_err(resp, format!("download {name}")).await
    }
}

/// PROPFIND directory listing: parse the `multistatus` XML body.
fn parse_multistatus(body: &[u8]) -> Result<Vec<RemoteEntry>> {
    let doc = roxmltree::Document::parse(
        std::str::from_utf8(body)
            .map_err(|e| AgentError::Other(format!("PROPFIND body is not UTF-8: {e}")))?,
    )
    .map_err(|e| AgentError::Other(format!("parse PROPFIND XML: {e}")))?;

    let mut entries = Vec::new();
    for resp in doc
        .descendants()
        .filter(|n| n.tag_name().name() == "response")
    {
        let mut name = None;
        let mut etag = None;
        let mut last_modified = None;
        let mut is_collection = false;
        for n in resp.descendants() {
            match n.tag_name().name() {
                // `<D:resourcetype><D:collection/>` marks the directory itself
                // — PROPFIND depth 1 always lists it, but it is not a synced
                // file. Filtering it here keeps `list()` as "the files in the
                // directory", which makes "remote dir is empty" decidable
                // (first-sync push-all detection, D002).
                "collection" => is_collection = true,
                "href" => {
                    if let Some(t) = n.text() {
                        name = Some(basename(t).to_string());
                    }
                }
                "getetag" => {
                    if let Some(t) = n.text() {
                        etag = Some(t.trim().to_string());
                    }
                }
                "getlastmodified" => {
                    if let Some(t) = n.text() {
                        last_modified = Some(t.to_string());
                    }
                }
                _ => {}
            }
        }
        if is_collection {
            continue;
        }
        if let Some(name) = name
            && !name.is_empty()
        {
            entries.push(RemoteEntry {
                name,
                etag,
                last_modified,
            });
        }
    }
    Ok(entries)
}

/// The last path segment of a URL (PROPFIND hrefs end with the resource name).
fn basename(href: &str) -> &str {
    href.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(href)
}

/// `dir_url + "/" + name`, tolerating a trailing slash on `dir_url`.
fn join_url(dir_url: &str, name: &str) -> String {
    if dir_url.ends_with('/') {
        format!("{dir_url}{name}")
    } else {
        format!("{dir_url}/{name}")
    }
}

fn network_err(e: reqwest::Error) -> AgentError {
    AgentError::Network(format!("webdav request: {e}"))
}

/// Map a non-success response to `Auth` (401/403) or a generic error.
async fn auth_or_err<T>(resp: reqwest::Response, what: impl AsRef<str>) -> Result<T> {
    let status = resp.status();
    let what = what.as_ref();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(AgentError::Auth(format!(
            "webdav {what} rejected ({status}); check the application password (everyday auth login --module webdav)"
        )));
    }
    Err(AgentError::Other(format!(
        "webdav {what} failed: HTTP {status}"
    )))
}

// ============ test seam: in-memory mock (test builds only) ============

/// In-memory WebDAV server for engine tests. Remote files live in a
/// `HashMap<name, bytes>`; ETags are SHA-256 of content (deterministic).
/// `files` is behind a mutex because the trait only exposes `&self`.
#[cfg(test)]
pub mod test_support {
    use super::*;

    #[derive(Default)]
    pub struct MockWebdavClient {
        /// Remote directory contents (name → raw bytes).
        pub files: std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>,
        /// Last-Modified value reported for every file (fixed, for LWW tests).
        pub fixed_last_modified: Option<String>,
        /// When true, every operation fails with a network error.
        pub fail_network: bool,
    }

    impl MockWebdavClient {
        /// A client whose files report the given `Last-Modified` (used to steer
        /// Last-Write-Wins arbitration in engine tests).
        pub fn with_last_modified(ts: &str) -> Self {
            Self {
                files: Default::default(),
                fixed_last_modified: Some(ts.to_string()),
                fail_network: false,
            }
        }

        /// Convenience: seed one remote file.
        pub fn seed(&self, name: &str, bytes: &[u8]) {
            self.files
                .lock()
                .unwrap()
                .insert(name.to_string(), bytes.to_vec());
        }
    }

    fn mock_etag(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(bytes);
        format!("\"{:x}\"", h.finalize())
    }

    #[async_trait]
    impl WebdavClient for MockWebdavClient {
        async fn ensure_dir(&self, _dir_url: &str) -> Result<()> {
            if self.fail_network {
                return Err(AgentError::Network("mock network down".into()));
            }
            Ok(())
        }

        async fn list(&self, _dir_url: &str) -> Result<Vec<RemoteEntry>> {
            if self.fail_network {
                return Err(AgentError::Network("mock network down".into()));
            }
            let guard = self.files.lock().unwrap();
            let mut out: Vec<RemoteEntry> = guard
                .iter()
                .map(|(name, bytes)| RemoteEntry {
                    name: name.clone(),
                    etag: Some(mock_etag(bytes)),
                    last_modified: self.fixed_last_modified.clone(),
                })
                .collect();
            out.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(out)
        }

        async fn put(&self, _dir_url: &str, name: &str, body: &[u8]) -> Result<RemoteEntry> {
            if self.fail_network {
                return Err(AgentError::Network("mock network down".into()));
            }
            self.files
                .lock()
                .unwrap()
                .insert(name.to_string(), body.to_vec());
            Ok(RemoteEntry {
                name: name.to_string(),
                etag: Some(mock_etag(body)),
                last_modified: self.fixed_last_modified.clone(),
            })
        }

        async fn get(&self, _dir_url: &str, name: &str) -> Result<Vec<u8>> {
            if self.fail_network {
                return Err(AgentError::Network("mock network down".into()));
            }
            self.files
                .lock()
                .unwrap()
                .get(name)
                .cloned()
                .ok_or_else(|| AgentError::Other(format!("mock remote file not found: {name}")))
        }
    }
} // mod test_support

#[cfg(test)]
mod tests {
    use super::test_support::MockWebdavClient;
    use super::*;

    #[test]
    fn basename_strips_directory_and_trailing_slash() {
        assert_eq!(basename("/dav/everyday/memory.db"), "memory.db");
        assert_eq!(basename("/dav/everyday/"), "everyday");
        assert_eq!(basename("memory.db"), "memory.db");
    }

    #[test]
    fn join_url_handles_trailing_slash() {
        assert_eq!(
            join_url("https://dav.example.com/everyday", "a.db"),
            "https://dav.example.com/everyday/a.db"
        );
        assert_eq!(
            join_url("https://dav.example.com/everyday/", "a.db"),
            "https://dav.example.com/everyday/a.db"
        );
    }

    #[test]
    fn parse_multistatus_extracts_entries() {
        let body = br#"<?xml version="1.0"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/dav/everyday/</D:href>
    <D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop></D:propstat>
  </D:response>
  <D:response>
    <D:href>/dav/everyday/memory.db</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag>"abc123"</D:getetag>
        <D:getlastmodified>Mon, 09 Aug 2026 12:00:00 GMT</D:getlastmodified>
      </D:prop>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/dav/everyday/config.toml</D:href>
    <D:propstat><D:prop><D:getetag>"def456"</D:getetag></D:prop></D:propstat>
  </D:response>
</D:multistatus>"#;
        let entries = parse_multistatus(body).unwrap();
        // The collection entry (the directory itself) is filtered out, so only
        // the two files remain.
        assert_eq!(entries.len(), 2);
        let mem = entries.iter().find(|e| e.name == "memory.db").unwrap();
        assert_eq!(mem.etag.as_deref(), Some("\"abc123\""));
        assert_eq!(
            mem.last_modified.as_deref(),
            Some("Mon, 09 Aug 2026 12:00:00 GMT")
        );
        let cfg = entries.iter().find(|e| e.name == "config.toml").unwrap();
        assert_eq!(cfg.etag.as_deref(), Some("\"def456\""));
        // The directory itself must never be listed as a file.
        assert!(entries.iter().all(|e| e.name != "everyday"));
    }

    #[tokio::test]
    async fn mock_client_roundtrip() {
        let c = MockWebdavClient::with_last_modified("Mon, 09 Aug 2026 12:00:00 GMT");
        c.ensure_dir("https://x/").await.unwrap();
        c.put("https://x/", "a.db", b"hello").await.unwrap();
        let list = c.list("https://x/").await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "a.db");
        assert_eq!(
            list[0].etag.as_deref(),
            Some("\"2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824\"")
        );
        assert_eq!(c.get("https://x/", "a.db").await.unwrap(), b"hello");
        assert!(c.get("https://x/", "nope.db").await.is_err());
    }

    #[tokio::test]
    async fn mock_client_network_failure() {
        let c = MockWebdavClient {
            fail_network: true,
            ..Default::default()
        };
        assert!(c.list("https://x/").await.is_err());
    }
}
