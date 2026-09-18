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
    /// `search <query>`: GLOB-match title / url / tags (OR-of-tokens).
    ///
    /// Deliberately the **same predicate** the cross-module
    /// `BookmarkSearchProvider` uses — one matcher, not two. Two implementations
    /// would drift, and `bookmark search Q` disagreeing with `everyday search Q`
    /// is a bug a user cannot explain.
    async fn search(&self, query: &str, limit: Option<usize>) -> Result<Vec<BookmarkItem>>;
}

/// Factory: single construction seam ([R016](../../../docs/adr/R016-action-backend-di.md)).
///
/// Since v0.13.0 ([R019](../../../docs/adr/R019-remove-notion-provider.md)) the
/// module is local-only, so `for_account` has exactly one concrete backend and no
/// keyring / provider branching.
pub fn for_account(account: &BookmarkAccount) -> Result<Box<dyn BookmarkBackend>> {
    Ok(Box::new(LocalBookmarkBackend::new(account.clone())))
}

/// The OR-of-tokens, case-insensitive substring rule that `bookmark search`
/// applies, in Rust.
///
/// The live implementation expresses this as SQL (`lower(col) GLOB '*token*'`
/// over title / url / tag); this is the same rule stated once for the in-memory
/// test backend. It exists so the mock does not hand-copy the predicate and
/// drift from it — the failure mode that motivated reusing
/// `search_for_search`'s matcher in the first place.
#[cfg(test)]
pub fn matches_query(item: &BookmarkItem, query: &str) -> bool {
    let tokens: Vec<String> = query
        .split_whitespace()
        .map(|t| t.to_ascii_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() {
        return false;
    }
    let title = item.title.to_ascii_lowercase();
    let url = item.url.to_ascii_lowercase();
    let tags: Vec<String> = item.tags.iter().map(|t| t.to_ascii_lowercase()).collect();
    tokens
        .iter()
        .any(|t| title.contains(t) || url.contains(t) || tags.iter().any(|tag| tag.contains(t)))
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

        /// Matches against the seed `items` using [`matches_query`], the same
        /// rule the SQL predicate encodes, so the action layer can be driven
        /// without SQLite and the two cannot silently diverge.
        async fn search(&self, query: &str, limit: Option<usize>) -> Result<Vec<BookmarkItem>> {
            let mut out: Vec<BookmarkItem> = self
                .items
                .iter()
                .filter(|it| matches_query(it, query))
                .cloned()
                .collect();
            if let Some(n) = limit {
                out.truncate(n);
            }
            Ok(out)
        }
    }
}
