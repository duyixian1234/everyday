//! `MemorySearchProvider` — current-state GLOB adapter.
//!
//! Per [K003](../../../docs/adr/K003-memory-searchable.md): tokens
//! whitespace-split, OR-merged, case-insensitive GLOB across
//! `(subject, predicate, object)` of `current_state_view`. `Hit.id`
//! format: `memory:<row_id>`. `Hit.title`: `"{s} {p} {o}"`. `Hit.snippet`
//! is empty (memory is structured; no prose to excerpt).
//!
//! No per-account concept (K004 single-instance). One global provider is
//! registered unconditionally.

use async_trait::async_trait;
use sqlx::Row;

use crate::config::Config;
use crate::error::Result;
use crate::modules::memory::store::{VIEW_CURRENT, open};
use crate::search::{Hit, SearchQuery, Searchable};

/// Per-module hard cap, matching other modules' local providers
/// ([S004](../../../docs/adr/S004-execution-model.md)).
const PER_MODULE_CAP: usize = 50;

#[derive(Default)]
pub struct MemorySearchProvider;

impl MemorySearchProvider {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Searchable for MemorySearchProvider {
    fn module_name(&self) -> &'static str {
        "memory"
    }

    async fn search(&self, q: &SearchQuery, _cfg: &Config) -> Result<Vec<Hit>> {
        // Build OR-of-tokens GLOB across subject/predicate/object.
        // Empty / all-metacharacter queries produce zero hits.
        let tokens: Vec<String> = q
            .tokens()
            .iter()
            .map(|t| t.to_ascii_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        // Reject any token with GLOB metacharacters — same conservative
        // rule as the other local providers ([S003] + [R008]). Memory is
        // append-only and stable, so a token with `*` is almost certainly
        // a user error rather than intent.
        if tokens.iter().any(|t| t.contains(['*', '?', '[', ']'])) {
            return Ok(Vec::new());
        }

        let mut patterns: Vec<String> = Vec::new();
        let mut conds: Vec<String> = Vec::new();
        for t in &tokens {
            patterns.push(format!("*{t}*"));
            let idx = patterns.len();
            conds.push(format!("lower(subject) GLOB ?{idx}"));
            patterns.push(format!("*{t}*"));
            let idx2 = patterns.len();
            conds.push(format!("lower(predicate) GLOB ?{idx2}"));
            patterns.push(format!("*{t}*"));
            let idx3 = patterns.len();
            conds.push(format!("lower(object) GLOB ?{idx3}"));
        }
        let where_clause = conds.join(" OR ");

        let cap = q.limit.unwrap_or(PER_MODULE_CAP);
        patterns.push(cap.to_string());
        let cap_idx = patterns.len();

        let sql = format!(
            "SELECT id, subject, predicate, object, created_at FROM {VIEW_CURRENT} \
             WHERE {where_clause} \
             ORDER BY created_at DESC LIMIT ?{cap_idx}"
        );

        let pool = open().await?;
        let mut query = sqlx::query(&sql);
        for p in &patterns {
            query = query.bind(p);
        }
        let rows = query.fetch_all(&pool).await?;

        let hits: Vec<Hit> = rows
            .iter()
            .map(|r| {
                let id: String = r.get("id");
                let s: String = r.get("subject");
                let p: String = r.get("predicate");
                let o: String = r.get("object");
                let created_at: String = r.get("created_at");
                let ts = crate::util::datetime::parse_rfc3339(&created_at);
                Hit {
                    module: "memory",
                    account: None,
                    // K003: id format lets the agent drill into `memory
                    // history` / `memory get` via id.
                    id: format!("memory:{id}"),
                    title: format!("{s} {p} {o}"),
                    // K003: snippet empty (memory is structured).
                    snippet: String::new(),
                    url: None,
                    ts,
                    // K003: kind matches the structural triple unit; not a
                    // prose page or message. Use "fact" so downstream UIs
                    // can distinguish.
                    kind: "fact",
                }
            })
            .collect();
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a temp pool and seed it with facts to exercise the provider.
    async fn seed_and_query() {
        use crate::modules::memory::store;
        let dir = std::env::temp_dir().join(format!("everyday-mem-search-{}", store::gen_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("memory.db");
        let _ = store::open_at(&path).await.unwrap();
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[tokio::test]
    async fn provider_module_name_is_memory() {
        let p = MemorySearchProvider::new();
        assert_eq!(p.module_name(), "memory");
    }

    #[tokio::test]
    async fn provider_smoke_seed_only() {
        // Sanity-check that the schema path is reachable; the full
        // GLOB-path tests live in mod.rs (where we can swap the
        // single-instance path).
        seed_and_query().await;
    }
}
