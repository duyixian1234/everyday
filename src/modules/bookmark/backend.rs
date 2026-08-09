//! Action-layer Backend trait + Dependency Inversion for the `bookmark` module ([R016](../../../docs/adr/R016-action-backend-di.md)).
//!
//! `BookmarkBackend` decouples the high-level action dispatch in `bookmark/mod.rs` from the
//! low-level provider protocol. The module never branches on `account.provider`,
//! and never touches the keyring — all of that lives in [`for_account`], the
//! single construction seam.
//!
//! Methods return **typed domain structs** (never `Output`); `bookmark/mod.rs` owns rendering
//! to text / `--json` ([R018](../../../docs/adr/R018-backend-domain-mocks.md)).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::config::BookmarkAccount;
use crate::error::Result;
use crate::modules::bookmark::local::LocalBookmarkBackend;

// ============ Domain types (R018) ============

/// Clean domain model (output to the Agent / terminal).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkItem {
    pub id: String,
    pub url: String,
    pub title: String,
    pub tags: Vec<String>,
}

/// Result of `add` (local bookmark). `database_id` (Notion-only) was removed
/// in v0.13.0 ([R019](../../../docs/adr/R019-remove-notion-provider.md)).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkAdded {
    pub id: String,
    pub url: String,
    pub title: String,
    pub tags: Vec<String>,
}

// ============ Trait + factory (R016) ============

#[async_trait]
pub trait BookmarkBackend: Send + Sync {
    async fn add(&self, url: &str, title: &str, tags: &[String]) -> Result<BookmarkAdded>;
    async fn list(&self, tag: Option<&str>) -> Result<Vec<BookmarkItem>>;
}

/// Factory: single construction seam ([R016](../../../docs/adr/R016-action-backend-di.md)).
///
/// Since v0.13.0 ([R019](../../../docs/adr/R019-remove-notion-provider.md)) the
/// module is local-only, so `for_account` has exactly one concrete backend and no
/// keyring / provider branching.
pub fn for_account(account: &BookmarkAccount) -> Result<Box<dyn BookmarkBackend>> {
    Ok(Box::new(LocalBookmarkBackend::new(account.clone())))
}

/// Test-only in-memory backend. Lives behind `#[cfg(test)]` so it never ships in the
/// binary. It holds pre-seeded domain data and returns it verbatim, letting the action
/// layer be exercised without SQLite — the DI acceptance guard for
/// [R016](../../../docs/adr/R016-action-backend-di.md) / [R018](../../../docs/adr/R018-backend-domain-mocks.md).
#[cfg(test)]
pub mod testkit {
    use super::*;
    use crate::error::AgentError;

    /// In-memory `BookmarkBackend`. `items` backs `list`; `added` backs `add`.
    /// Missing fields error, mirroring a real backend that was never given the
    /// data to respond with.
    #[derive(Clone, Default)]
    pub struct MockBookmarkBackend {
        pub items: Vec<BookmarkItem>,
        pub added: Option<BookmarkAdded>,
    }

    #[async_trait]
    impl BookmarkBackend for MockBookmarkBackend {
        async fn add(&self, _url: &str, _title: &str, _tags: &[String]) -> Result<BookmarkAdded> {
            self.added
                .clone()
                .ok_or_else(|| AgentError::Other("mock added unset".into()))
        }

        async fn list(&self, _tag: Option<&str>) -> Result<Vec<BookmarkItem>> {
            Ok(self.items.clone())
        }
    }
}
