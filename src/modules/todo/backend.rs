//! Action-layer Backend trait + Dependency Inversion for the `todo` module ([R016](../../../docs/adr/R016-action-backend-di.md)).
//!
//! `TodoBackend` decouples the high-level action dispatch in `todo/mod.rs` from the
//! low-level provider protocol. The module never branches on `account.provider`,
//! and never touches the keyring — all of that lives in [`for_account`], the
//! single construction seam.
//!
//! Methods return **typed domain structs** (never `Output`); `todo/mod.rs` owns rendering
//! to text / `--json` ([R018](../../../docs/adr/R018-backend-domain-mocks.md)).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::config::TodoAccount;
use crate::error::Result;
use crate::modules::todo::local::LocalTodoBackend;

// ============ Status constants ============

/// Status option names (must match the schema created on first use).
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

/// Result of `add` (local todo). `url` / `database_id` (Notion-only) were
/// removed in v0.13.0 ([R019](../../../docs/adr/R019-remove-notion-provider.md)).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoAdded {
    pub id: String,
    pub title: String,
}

/// Result of `start` / `complete`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoStatusSet {
    pub id: String,
    pub status: String,
}

/// Result of `delete` (local physical delete; the Notion `archived` flag was removed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoDeleted {
    pub id: String,
    pub title: String,
    pub status: String,
}

// ============ Trait + factory (R016) ============

#[async_trait]
pub trait TodoBackend: Send + Sync {
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

/// Factory: single construction seam ([R016](../../../docs/adr/R016-action-backend-di.md)).
///
/// Since v0.13.0 ([R019](../../../docs/adr/R019-remove-notion-provider.md)) the
/// module is local-only, so `for_account` has exactly one concrete backend and no
/// keyring / provider branching.
pub fn for_account(account: &TodoAccount) -> Result<Box<dyn TodoBackend>> {
    Ok(Box::new(LocalTodoBackend::new(account.clone())))
}

/// Test-only in-memory backend. Lives behind `#[cfg(test)]` so it never ships in the
/// binary. It holds pre-seeded domain data and returns it verbatim, letting the action
/// layer be exercised without SQLite — the DI acceptance guard for
/// [R016](../../../docs/adr/R016-action-backend-di.md) / [R018](../../../docs/adr/R018-backend-domain-mocks.md).
#[cfg(test)]
pub mod testkit {
    use super::*;
    use crate::error::AgentError;

    /// In-memory `TodoBackend`. `items` backs `list`; the `added` / `status_set` /
    /// `deleted` fields back their respective actions. Missing fields error, mirroring a real
    /// backend that was never given the data to respond with.
    #[derive(Clone, Default)]
    pub struct MockTodoBackend {
        pub items: Vec<TodoItem>,
        pub added: Option<TodoAdded>,
        pub status_set: Option<TodoStatusSet>,
        pub deleted: Option<TodoDeleted>,
    }

    #[async_trait]
    impl TodoBackend for MockTodoBackend {
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
