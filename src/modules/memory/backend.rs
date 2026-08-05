//! Memory service trait + factory (P1 wiring, [F012](../../../docs/adr/F012-architecture-deepening-phase.md)).
//!
//! `actions.rs` is the pure domain layer: seven `async fn` returning typed
//! domain structs (never `Output`). `MemoryBackend` formalizes that layer as a
//! trait so the module's `execute` becomes a thin parameter-parser + renderer
//! (`dispatch`), and dispatch logic can be tested against an in-memory
//! [`testkit::MockMemoryBackend`] without touching `~/.config/everyday/memory.db`.
//!
//! The real backend is stateless: it delegates to `actions::*`, which own the
//! connection-pool lifecycle (`store::open` per call — the single-instance
//! design, [K004](../../../docs/adr/K004-memory-single-instance.md)).

use async_trait::async_trait;

use crate::error::Result;
use crate::modules::memory::actions::{self, AddResult, DeleteResult, GraphNode, QueryResult};

/// Memory service trait: domain methods, no `Output` in sight (P1).
///
/// `MemoryBackendImpl` delegates to `actions::*`; `testkit::MockMemoryBackend`
/// (tests) returns fixed data. `dispatch` in `super::mod` is the only place
/// that maps CLI args → service calls → `Output`.
#[async_trait]
pub trait MemoryBackend: Send + Sync {
    /// `memory add <S> <P> <O> [--confidence N] [--source LABEL]`.
    /// `confidence` is the raw string form (parsed by the action layer).
    async fn add(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        confidence: Option<&str>,
        source: Option<&str>,
    ) -> Result<AddResult>;

    /// `memory get <SUBJECT>`: current state of all triples with this subject.
    async fn get(&self, subject: &str) -> Result<QueryResult>;

    /// `memory relation <SUBJECT> <PREDICATE>`: current state of matching triples.
    async fn relation(&self, subject: &str, predicate: &str) -> Result<QueryResult>;

    /// `memory list [--limit N]`: all current-state rows, capped at N.
    async fn list(&self, limit: Option<usize>) -> Result<QueryResult>;

    /// `memory delete <S> <P> <O>`: soft-delete the current-state row.
    async fn delete(&self, subject: &str, predicate: &str, object: &str) -> Result<DeleteResult>;

    /// `memory graph <SUBJECT> [--depth N] [--include-deleted]`: forward BFS tree.
    async fn graph(
        &self,
        subject: &str,
        depth: Option<u8>,
        include_deleted: bool,
    ) -> Result<GraphNode>;

    /// `memory history <S> <P> <O>`: full version history (incl. deleted).
    async fn history(&self, subject: &str, predicate: &str, object: &str) -> Result<QueryResult>;
}

/// Real backend: delegates to the action handlers in `actions.rs`.
pub struct MemoryBackendImpl;

#[async_trait]
impl MemoryBackend for MemoryBackendImpl {
    async fn add(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        confidence: Option<&str>,
        source: Option<&str>,
    ) -> Result<AddResult> {
        actions::add(subject, predicate, object, confidence, source).await
    }

    async fn get(&self, subject: &str) -> Result<QueryResult> {
        actions::get(subject).await
    }

    async fn relation(&self, subject: &str, predicate: &str) -> Result<QueryResult> {
        actions::relation(subject, predicate).await
    }

    async fn list(&self, limit: Option<usize>) -> Result<QueryResult> {
        actions::list(limit).await
    }

    async fn delete(&self, subject: &str, predicate: &str, object: &str) -> Result<DeleteResult> {
        actions::delete(subject, predicate, object).await
    }

    async fn graph(
        &self,
        subject: &str,
        depth: Option<u8>,
        include_deleted: bool,
    ) -> Result<GraphNode> {
        actions::graph(subject, depth, include_deleted).await
    }

    async fn history(&self, subject: &str, predicate: &str, object: &str) -> Result<QueryResult> {
        actions::history(subject, predicate, object).await
    }
}

/// Build the default backend (single global instance, [K004](../../../docs/adr/K004-memory-single-instance.md)).
pub fn for_default() -> Box<dyn MemoryBackend> {
    Box::new(MemoryBackendImpl)
}

/// Test-only in-memory backend. Lives behind `#[cfg(test)]` so it never ships
/// in the binary. Records the last invocation so dispatch tests can assert
/// that CLI flags were parsed and forwarded correctly — the DI acceptance
/// guard for the P1 wiring ([F012](../../../docs/adr/F012-architecture-deepening-phase.md)).
#[cfg(test)]
pub mod testkit {
    use super::*;
    use std::sync::Mutex;

    /// In-memory `MemoryBackend`. `facts` backs get/relation/list/history;
    /// `add`/`delete`/`graph` synthesize fixed domain results and record the
    /// call parameters for assertion.
    #[derive(Default)]
    pub struct MockMemoryBackend {
        /// Facts returned by get/relation/list/history.
        pub facts: Vec<actions::MemoryFact>,
        /// `(subject, predicate, object)` of the last `add` call.
        pub last_add: Mutex<Option<(String, String, String)>>,
        /// `confidence`/`source` strings of the last `add` call.
        pub last_add_meta: Mutex<Option<(Option<String>, Option<String>)>>,
        /// Last `list` limit.
        pub last_list_limit: Mutex<Option<usize>>,
    }

    #[async_trait]
    impl MemoryBackend for MockMemoryBackend {
        async fn add(
            &self,
            subject: &str,
            predicate: &str,
            object: &str,
            confidence: Option<&str>,
            source: Option<&str>,
        ) -> Result<AddResult> {
            let mut f = self
                .facts
                .iter()
                .find(|f| f.subject == subject && f.predicate == predicate && f.object == object)
                .cloned()
                .unwrap_or_else(|| actions::MemoryFact {
                    id: "mock-id".to_string(),
                    subject: subject.to_string(),
                    predicate: predicate.to_string(),
                    object: object.to_string(),
                    confidence: 1.0,
                    source: None,
                    created_at: "2026-08-05T00:00:00Z".to_string(),
                    deleted_at: None,
                });
            f.confidence = confidence
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(1.0);
            f.source = source.map(|s| s.to_string());
            *self.last_add.lock().unwrap() = Some((
                subject.to_string(),
                predicate.to_string(),
                object.to_string(),
            ));
            *self.last_add_meta.lock().unwrap() = Some((
                confidence.map(|s| s.to_string()),
                source.map(|s| s.to_string()),
            ));
            Ok(f)
        }

        async fn get(&self, subject: &str) -> Result<QueryResult> {
            Ok(QueryResult {
                facts: self
                    .facts
                    .iter()
                    .filter(|f| f.subject == subject)
                    .cloned()
                    .collect(),
            })
        }

        async fn relation(&self, subject: &str, predicate: &str) -> Result<QueryResult> {
            Ok(QueryResult {
                facts: self
                    .facts
                    .iter()
                    .filter(|f| f.subject == subject && f.predicate == predicate)
                    .cloned()
                    .collect(),
            })
        }

        async fn list(&self, limit: Option<usize>) -> Result<QueryResult> {
            *self.last_list_limit.lock().unwrap() = limit;
            let mut facts = self.facts.clone();
            if let Some(n) = limit {
                facts.truncate(n);
            }
            Ok(QueryResult { facts })
        }

        async fn delete(
            &self,
            subject: &str,
            predicate: &str,
            object: &str,
        ) -> Result<DeleteResult> {
            Ok(DeleteResult {
                id: "mock-del-id".to_string(),
                subject: subject.to_string(),
                predicate: predicate.to_string(),
                object: object.to_string(),
                deleted_at: "2026-08-05T00:00:00Z".to_string(),
            })
        }

        async fn graph(
            &self,
            subject: &str,
            _depth: Option<u8>,
            _include_deleted: bool,
        ) -> Result<GraphNode> {
            Ok(GraphNode {
                subject: subject.to_string(),
                predicates: vec![],
            })
        }

        async fn history(
            &self,
            subject: &str,
            predicate: &str,
            object: &str,
        ) -> Result<QueryResult> {
            Ok(QueryResult {
                facts: self
                    .facts
                    .iter()
                    .filter(|f| {
                        f.subject == subject && f.predicate == predicate && f.object == object
                    })
                    .cloned()
                    .collect(),
            })
        }
    }
}
