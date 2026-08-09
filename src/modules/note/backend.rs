//! Action-layer Backend trait + Dependency Inversion for the `note` module ([R016](../../../docs/adr/R016-action-backend-di.md)).
//!
//! `NoteBackend` decouples the high-level action dispatch in `note/mod.rs` from the
//! low-level provider protocol. The module never branches on `account.provider`,
//! and never touches the keyring — all of that lives in [`for_account`], the
//! single construction seam.
//!
//! Methods return **typed domain structs** (never `Output`); `note/mod.rs` owns rendering
//! to text / `--json` ([R018](../../../docs/adr/R018-backend-domain-mocks.md)).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config::NoteAccount;
use crate::error::Result;
use crate::modules::note::local::LocalNoteBackend;

// ============ Domain types (R018) ============

/// A single search / list row. Local-only since v0.13.0
/// ([R019](../../../docs/adr/R019-remove-notion-provider.md)) — the Notion
/// object-type `kind` discriminator was removed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteSummary {
    pub id: String,
    pub title: String,
    pub updated: String,
}

/// A list row, carrying the page's simplified property map.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteListEntry {
    pub id: String,
    pub title: String,
    pub updated: String,
    pub properties: Map<String, Value>,
}

/// Result of `create` (local note).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteCreated {
    pub id: String,
    pub title: String,
    pub prop_count: usize,
}

/// Result of `read`: body aggregated into Markdown plus the simplified property map.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteRead {
    pub id: String,
    pub title: String,
    pub properties: Map<String, Value>,
    pub content: String,
}

/// Result of `append` (local note: appended line count).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteAppended {
    pub id: String,
    pub appended: usize,
}

/// Result of `update`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteUpdated {
    pub id: String,
    pub updated_count: usize,
}

// ============ Trait + factory (R016) ============

#[async_trait]
pub trait NoteBackend: Send + Sync {
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<NoteSummary>>;
    async fn list(&self, limit: usize) -> Result<Vec<NoteListEntry>>;
    async fn create(&self, title: &str, props: &[(String, String)]) -> Result<NoteCreated>;
    async fn read(&self, page_id: &str) -> Result<NoteRead>;
    async fn append(&self, page_id: &str, text: &str) -> Result<NoteAppended>;
    async fn update(&self, page_id: &str, props: &[(String, String)]) -> Result<NoteUpdated>;
}

/// Factory: single construction seam ([R016](../../../docs/adr/R016-action-backend-di.md)).
///
/// Since v0.13.0 ([R019](../../../docs/adr/R019-remove-notion-provider.md)) the
/// module is local-only, so `for_account` has exactly one concrete backend and no
/// keyring / provider branching. The signature is kept so the action layer stays
/// backend-agnostic (and testable via `MockNoteBackend`).
pub fn for_account(account: &NoteAccount) -> Result<Box<dyn NoteBackend>> {
    Ok(Box::new(LocalNoteBackend::new(account.clone())))
}

/// Test-only in-memory backend. Lives behind `#[cfg(test)]` so it never ships in the
/// binary. It holds pre-seeded domain data and returns it verbatim, letting the action
/// layer be exercised without SQLite — the DI acceptance guard for
/// [R016](../../../docs/adr/R016-action-backend-di.md) / [R018](../../../docs/adr/R018-backend-domain-mocks.md).
#[cfg(test)]
pub mod testkit {
    use super::*;
    use crate::error::AgentError;
    use std::collections::HashMap;

    /// In-memory `NoteBackend`. `summaries`/`entries` back `search`/`list`; `pages` backs
    /// `read` by id. `create`/`append`/`update` synthesize domain results from their inputs.
    #[derive(Clone, Default)]
    pub struct MockNoteBackend {
        /// Rows returned by `search`.
        pub summaries: Vec<NoteSummary>,
        /// Rows returned by `list`.
        pub entries: Vec<NoteListEntry>,
        /// Pages keyed by id, returned by `read`.
        pub pages: HashMap<String, NoteRead>,
    }

    #[async_trait]
    impl NoteBackend for MockNoteBackend {
        async fn search(&self, _query: &str, _limit: usize) -> Result<Vec<NoteSummary>> {
            Ok(self.summaries.clone())
        }

        async fn list(&self, _limit: usize) -> Result<Vec<NoteListEntry>> {
            Ok(self.entries.clone())
        }

        async fn create(&self, title: &str, props: &[(String, String)]) -> Result<NoteCreated> {
            Ok(NoteCreated {
                id: "mock-page".to_string(),
                title: title.to_string(),
                prop_count: props.len(),
            })
        }

        async fn read(&self, page_id: &str) -> Result<NoteRead> {
            self.pages
                .get(page_id)
                .cloned()
                .ok_or_else(|| AgentError::Other(format!("mock note {page_id} not found")))
        }

        async fn append(&self, page_id: &str, text: &str) -> Result<NoteAppended> {
            let appended = text
                .split('\n')
                .filter(|l| !l.trim().is_empty())
                .count()
                .max(1);
            Ok(NoteAppended {
                id: page_id.to_string(),
                appended,
            })
        }

        async fn update(&self, page_id: &str, props: &[(String, String)]) -> Result<NoteUpdated> {
            Ok(NoteUpdated {
                id: page_id.to_string(),
                updated_count: props.len(),
            })
        }
    }
}
