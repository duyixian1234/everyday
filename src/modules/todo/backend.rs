//! Action-layer Backend trait + Dependency Inversion for the `todo` module ([R016](../../../docs/adr/R016-action-backend-di.md)).
//!
//! `TodoBackend` decouples the high-level action dispatch in `todo/mod.rs` from the
//! low-level provider protocol. The module never names `NotionClient`, never branches on
//! `account.provider`, and never touches the keyring — all of that lives in
//! [`for_account`], the single construction seam.
//!
//! Methods return **typed domain structs** (never `Output`); `todo/mod.rs` owns rendering
//! to text / `--json` ([R018](../../../docs/adr/R018-backend-domain-mocks.md)).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::config::{Config, TodoAccount};
use crate::error::Result;
use crate::modules::auth;
use crate::modules::local::is_local_provider;
use crate::modules::todo::local::LocalTodoBackend;
use crate::modules::todo::notion::NotionTodoBackend;
use crate::notion_client::NotionClient;

// ============ Status constants (shared by both providers) ============

/// Status option names (must match the schema created by `init-db`).
pub const STATUS_TODO: &str = "Todo";
pub const STATUS_IN_PROGRESS: &str = "In Progress";
pub const STATUS_DONE: &str = "Done";

// ============ Domain types (R018) ============

/// Clean domain model (output to the Agent / terminal).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub title: String,
    pub status: String,
    pub due: Option<String>,
    pub priority: Option<String>,
}

/// Result of `init-db`. `db_path` is `Some` for the local provider; `database_id` /
/// `url` are `Some` for the Notion provider (which also writes `database_id` back to config).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoInitDb {
    pub account: String,
    pub provider: &'static str,
    pub db_path: Option<String>,
    pub database_id: Option<String>,
    pub url: Option<String>,
}

/// Result of `add`. `url` / `database_id` are `Some` for the Notion provider, `None` for local.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoAdded {
    pub id: String,
    pub url: Option<String>,
    pub title: String,
    pub database_id: Option<String>,
}

/// Result of `start` / `complete`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoStatusSet {
    pub id: String,
    pub status: String,
    pub url: Option<String>,
}

/// Result of `delete`. `archived` is `false` for the local provider (physical delete)
/// and `true` for the Notion provider (soft archive).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoDeleted {
    pub id: String,
    pub title: String,
    pub status: String,
    pub archived: bool,
}

// ============ Trait + factory (R016) ============

#[async_trait]
pub trait TodoBackend: Send + Sync {
    async fn init_db(&self, parent: Option<&str>) -> Result<TodoInitDb>;
    async fn list(&self, all: bool) -> Result<Vec<TodoItem>>;
    async fn add(
        &self,
        title: &str,
        due: Option<&str>,
        priority: Option<&str>,
    ) -> Result<TodoAdded>;
    async fn set_status(&self, id: &str, status: &str) -> Result<TodoStatusSet>;
    async fn delete(&self, id: &str) -> Result<TodoDeleted>;
}

/// Factory: centralizes provider selection + token fetch ([R016](../../../docs/adr/R016-action-backend-di.md)).
///
/// The module's action code calls only this; it never names `NotionClient`, never
/// branches on provider, never touches the keyring. The `NotionClient` is constructed
/// exactly once here (not per action). Returns a `Box<dyn TodoBackend>` so the caller
/// stays provider-agnostic. The Notion backend's `init_db` writes `database_id` back to
/// config via the static `Config::config_path()` — that side effect is an implementation
/// detail hidden inside the provider, not a branch in the module.
pub fn for_account(config: &Config, account: &TodoAccount) -> Result<Box<dyn TodoBackend>> {
    if is_local_provider(&account.provider) {
        Ok(Box::new(LocalTodoBackend::new(account.clone())))
    } else {
        let token = auth::get_credential(config, "todo", &account.name)?;
        let client = NotionClient::new(token)?;
        Ok(Box::new(NotionTodoBackend::new(client, account.clone())))
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

    /// In-memory `TodoBackend`. `items` backs `list`; the `added` / `status_set` / `deleted` /
    /// `init_db` fields back their respective actions. Missing fields error, mirroring a real
    /// backend that was never given the data to respond with.
    #[derive(Clone, Default)]
    pub struct MockTodoBackend {
        pub items: Vec<TodoItem>,
        pub added: Option<TodoAdded>,
        pub status_set: Option<TodoStatusSet>,
        pub deleted: Option<TodoDeleted>,
        pub init_db: Option<TodoInitDb>,
    }

    #[async_trait]
    impl TodoBackend for MockTodoBackend {
        async fn init_db(&self, _parent: Option<&str>) -> Result<TodoInitDb> {
            self.init_db
                .clone()
                .ok_or_else(|| AgentError::Other("mock init_db unset".into()))
        }

        async fn list(&self, _all: bool) -> Result<Vec<TodoItem>> {
            Ok(self.items.clone())
        }

        async fn add(
            &self,
            _title: &str,
            _due: Option<&str>,
            _priority: Option<&str>,
        ) -> Result<TodoAdded> {
            self.added
                .clone()
                .ok_or_else(|| AgentError::Other("mock added unset".into()))
        }

        async fn set_status(&self, _id: &str, _status: &str) -> Result<TodoStatusSet> {
            self.status_set
                .clone()
                .ok_or_else(|| AgentError::Other("mock status_set unset".into()))
        }

        async fn delete(&self, _id: &str) -> Result<TodoDeleted> {
            self.deleted
                .clone()
                .ok_or_else(|| AgentError::Other("mock deleted unset".into()))
        }
    }
}
