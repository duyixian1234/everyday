//! Action-layer Backend trait + Dependency Inversion for the `bookmark` module ([R016](../../../docs/adr/R016-action-backend-di.md)).
//!
//! `BookmarkBackend` decouples the high-level action dispatch in `bookmark/mod.rs` from the
//! low-level provider protocol. The module never names `NotionClient`, never branches on
//! `account.provider`, and never touches the keyring — all of that lives in
//! [`for_account`], the single construction seam.
//!
//! Methods return **typed domain structs** (never `Output`); `bookmark/mod.rs` owns rendering
//! to text / `--json` ([R018](../../../docs/adr/R018-backend-domain-mocks.md)).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::config::{BookmarkAccount, Config};
use crate::error::Result;
use crate::modules::auth;
use crate::modules::bookmark::local::LocalBookmarkBackend;
use crate::modules::bookmark::notion::NotionBookmarkBackend;
use crate::modules::local::is_local_provider;
use crate::notion_client::NotionClient;

// ============ Domain types (R018) ============

/// Clean domain model (output to the Agent / terminal).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkItem {
    pub id: String,
    pub url: String,
    pub title: String,
    pub tags: Vec<String>,
}

/// Result of `init-db`. `db_path` is `Some` for the local provider; `database_id` /
/// `url` are `Some` for the Notion provider (which also writes `database_id` back to config).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkInitDb {
    pub account: String,
    pub provider: &'static str,
    pub db_path: Option<String>,
    pub database_id: Option<String>,
    pub url: Option<String>,
}

/// Result of `add`. `database_id` is `Some` for the Notion provider, `None` for local.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkAdded {
    pub id: String,
    pub url: String,
    pub title: String,
    pub tags: Vec<String>,
    pub database_id: Option<String>,
}

// ============ Trait + factory (R016) ============

#[async_trait]
pub trait BookmarkBackend: Send + Sync {
    async fn init_db(&self, parent: Option<&str>) -> Result<BookmarkInitDb>;
    async fn add(
        &self,
        url: &str,
        title: &str,
        tags: &[String],
        db_id: Option<&str>,
    ) -> Result<BookmarkAdded>;
    async fn list(&self, tag: Option<&str>, db_id: Option<&str>) -> Result<Vec<BookmarkItem>>;
}

/// Factory: centralizes provider selection + token fetch ([R016](../../../docs/adr/R016-action-backend-di.md)).
///
/// The module's action code calls only this; it never names `NotionClient`, never
/// branches on provider, never touches the keyring. The `NotionClient` is constructed
/// exactly once here (not per action). Returns a `Box<dyn BookmarkBackend>` so the caller
/// stays provider-agnostic. The Notion backend's `init_db` writes `database_id` back to
/// config via the static `Config::config_path()` — that side effect is an implementation
/// detail hidden inside the provider, not a branch in the module.
pub fn for_account(config: &Config, account: &BookmarkAccount) -> Result<Box<dyn BookmarkBackend>> {
    if is_local_provider(&account.provider) {
        Ok(Box::new(LocalBookmarkBackend::new(account.clone())))
    } else {
        let token = auth::get_credential(config, "bookmark", &account.name)?;
        let client = NotionClient::new(token)?;
        Ok(Box::new(NotionBookmarkBackend::new(
            client,
            account.clone(),
        )))
    }
}

/// Test-only in-memory backend. Lives behind `#[cfg(test)]` so it never ships in the
/// binary. It holds pre-seeded domain data and returns it verbatim, letting the action
/// layer be exercised without a `NotionClient` or SQLite — the DI acceptance guard for
/// [R016](../../../docs/adr/R016-action-backend-di.md) / [R018](../../../docs/adr/R018-backend-domain-mocks.md).
#[cfg(test)]
pub mod testkit {
    use super::*;
    use crate::error::AgentError;

    /// In-memory `BookmarkBackend`. `items` backs `list`; `added` / `init_db` back their
    /// respective actions. Missing fields error, mirroring a real backend that was never
    /// given the data to respond with.
    #[derive(Clone, Default)]
    pub struct MockBookmarkBackend {
        pub items: Vec<BookmarkItem>,
        pub added: Option<BookmarkAdded>,
        pub init_db: Option<BookmarkInitDb>,
    }

    #[async_trait]
    impl BookmarkBackend for MockBookmarkBackend {
        async fn init_db(&self, _parent: Option<&str>) -> Result<BookmarkInitDb> {
            self.init_db
                .clone()
                .ok_or_else(|| AgentError::Other("mock init_db unset".into()))
        }

        async fn add(
            &self,
            _url: &str,
            _title: &str,
            _tags: &[String],
            _db_id: Option<&str>,
        ) -> Result<BookmarkAdded> {
            self.added
                .clone()
                .ok_or_else(|| AgentError::Other("mock added unset".into()))
        }

        async fn list(
            &self,
            _tag: Option<&str>,
            _db_id: Option<&str>,
        ) -> Result<Vec<BookmarkItem>> {
            Ok(self.items.clone())
        }
    }
}
