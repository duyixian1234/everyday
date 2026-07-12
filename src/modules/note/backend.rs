//! Action-layer Backend trait + Dependency Inversion for the `note` module ([R016](../../../docs/adr/R016-action-backend-di.md)).
//!
//! `NoteBackend` decouples the high-level action dispatch in `note/mod.rs` from the
//! low-level provider protocol. The module never names `NotionClient`, never branches on
//! `account.provider`, and never touches the keyring — all of that lives in
//! [`for_account`], the single construction seam.
//!
//! Methods return **typed domain structs** (never `Output`); `note/mod.rs` owns rendering
//! to text / `--json` ([R018](../../../docs/adr/R018-backend-domain-mocks.md)).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config::{Config, NoteAccount};
use crate::error::Result;
use crate::modules::auth;
use crate::modules::local::is_local_provider;
use crate::modules::note::local::LocalNoteBackend;
use crate::modules::note::notion::NotionNoteBackend;
use crate::notion_client::NotionClient;

// ============ Domain types (R018) ============

/// A single search / list row. `kind` is the Notion object type (`page` / `database`);
/// `properties` is `None` for `search` and `Some(..)` for `list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteSummary {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub url: Option<String>,
    pub updated: String,
}

/// A list row, carrying the page's simplified property map.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteListEntry {
    pub id: String,
    pub title: String,
    pub url: Option<String>,
    pub updated: String,
    pub properties: Map<String, Value>,
}

/// Result of `create`: `database_id` is `Some` for the Notion provider, `None` for local.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteCreated {
    pub id: String,
    pub title: String,
    pub url: Option<String>,
    pub database_id: Option<String>,
    pub prop_count: usize,
    pub resource: &'static str,
}

/// Result of `read`: body aggregated into Markdown plus the simplified property map.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteRead {
    pub id: String,
    pub title: String,
    pub url: Option<String>,
    pub properties: Map<String, Value>,
    pub content: String,
}

/// Result of `append`: `unit` / `resource` discriminate block(s)/page (Notion) from
/// line(s)/note (local) so the module can render an identical message shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteAppended {
    pub id: String,
    pub url: Option<String>,
    pub appended: usize,
    pub resource: &'static str,
    pub unit: &'static str,
}

/// Result of `update`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteUpdated {
    pub id: String,
    pub url: Option<String>,
    pub updated_count: usize,
    pub resource: &'static str,
}

// ============ Trait + factory (R016) ============

#[async_trait]
pub trait NoteBackend: Send + Sync {
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<NoteSummary>>;
    async fn list(&self, db_id: Option<&str>, limit: usize) -> Result<Vec<NoteListEntry>>;
    async fn create(
        &self,
        title: &str,
        db_id: Option<&str>,
        props: &[(String, String)],
    ) -> Result<NoteCreated>;
    async fn read(&self, page_id: &str) -> Result<NoteRead>;
    async fn append(&self, page_id: &str, text: &str) -> Result<NoteAppended>;
    async fn update(&self, page_id: &str, props: &[(String, String)]) -> Result<NoteUpdated>;
}

/// Factory: centralizes provider selection + token fetch ([R016](../../../docs/adr/R016-action-backend-di.md)).
///
/// The module's action code calls only this; it never names `NotionClient`, never
/// branches on provider, never touches the keyring. The `NotionClient` is constructed
/// exactly once here (not per action). Returns a `Box<dyn NoteBackend>` so the caller
/// stays provider-agnostic.
pub fn for_account(config: &Config, account: &NoteAccount) -> Result<Box<dyn NoteBackend>> {
    if is_local_provider(&account.provider) {
        Ok(Box::new(LocalNoteBackend::new(account.clone())))
    } else {
        let token = auth::get_credential(config, "note", &account.name)?;
        let client = NotionClient::new(token)?;
        Ok(Box::new(NotionNoteBackend::new(client)))
    }
}
