//! Memory action handlers: add / get / relation / list / delete / graph / history.
//!
//! Each handler returns a typed domain struct. Rendering happens in
//! [`super::mod`] (the module layer). This split mirrors the
//! `LocalNoteBackend` / `dispatch` separation
//! ([R016](../../../docs/adr/R016-action-backend-di.md)).
//!
//! All commands share the same connection pool from [`store::open`]. The
//! current-state view [`store::VIEW_CURRENT`] is the single source of
//! truth for "what is current".
//!
//! # Graph traversal (K002)
//!
//! Forward-only BFS on current state, default depth 2 max 5. Cycle
//! detection via visited set keyed by `(subject, predicate, object)` of
//! rendered edges. Recursion bounded, output size predictable.

use std::collections::HashSet;

use serde::Serialize;
use sqlx::{Row, SqlitePool};

use crate::error::{AgentError, Result};
use crate::modules::memory::store::{TABLE, VIEW_CURRENT, gen_id, open, parse_confidence};

/// Maximum recursion depth for `memory graph`. Per [K002](../../../docs/adr/K002-memory-graph-query.md).
pub const GRAPH_MAX_DEPTH: u8 = 5;

/// Default depth for `memory graph`. Per K002.
pub const GRAPH_DEFAULT_DEPTH: u8 = 2;

/// Default cap for `memory list`. Per K001, default 100.
pub const LIST_DEFAULT_LIMIT: usize = 100;

/// Domain struct: one (current or historical) memory row.
///
/// `deleted_at` is `None` for current-state rows and populated only for
/// `history` output where deleted rows are surfaced explicitly.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryFact {
    pub id: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f64,
    pub source: Option<String>,
    pub created_at: String,
    pub deleted_at: Option<String>,
}

/// Result of `memory add`: the freshly inserted row.
pub type AddResult = MemoryFact;

/// Result of `memory get` / `memory relation` / `memory list`: a flat
/// list of current-state facts.
#[derive(Debug, Clone, Serialize)]
pub struct QueryResult {
    pub facts: Vec<MemoryFact>,
}

/// Result of `memory delete`: which row was soft-deleted (with its `id`).
#[derive(Debug, Clone, Serialize)]
pub struct DeleteResult {
    pub id: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub deleted_at: String,
}

/// Recursive graph node — used for `memory graph` text + JSON output.
///
/// `predicates` groups outgoing edges by predicate name; `objects` is the
/// list of (object → its own sub-tree).
#[derive(Debug, Clone, Serialize)]
pub struct GraphNode {
    pub subject: String,
    pub predicates: Vec<GraphEdge>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphEdge {
    pub name: String,
    pub objects: Vec<GraphObject>,
}

/// One object node: carries the object value plus any further traversal.
#[derive(Debug, Clone, Serialize)]
pub struct GraphObject {
    pub name: String,
    pub predicates: Vec<GraphEdge>,
}

// ============ add ============

/// `memory add <S> <P> <O> [--confidence N] [--source LABEL]`
pub async fn add(
    subject: &str,
    predicate: &str,
    object: &str,
    confidence: Option<&str>,
    source: Option<&str>,
) -> Result<AddResult> {
    let pool = open().await?;
    let id = gen_id();
    let now = chrono::Utc::now().to_rfc3339();
    let conf = match confidence {
        Some(s) => parse_confidence(s)?,
        None => 1.0_f64,
    };
    let src = source.map(|s| s.to_string());

    sqlx::query(
        "INSERT INTO memory (id, subject, predicate, object, confidence, source, created_at, deleted_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
    )
    .bind(&id)
    .bind(subject)
    .bind(predicate)
    .bind(object)
    .bind(conf)
    .bind(&src)
    .bind(&now)
    .execute(&pool)
    .await?;

    Ok(MemoryFact {
        id,
        subject: subject.to_string(),
        predicate: predicate.to_string(),
        object: object.to_string(),
        confidence: conf,
        source: src,
        created_at: now,
        deleted_at: None,
    })
}

// ============ get / relation / list ============

/// `memory get <SUBJECT>`: current state of all triples with this subject.
pub async fn get(subject: &str) -> Result<QueryResult> {
    let pool = open().await?;
    let rows = sqlx::query(&format!(
        "SELECT id, subject, predicate, object, confidence, source, created_at \
         FROM {VIEW_CURRENT} WHERE subject = ?1 ORDER BY created_at DESC"
    ))
    .bind(subject)
    .fetch_all(&pool)
    .await?;
    let facts = rows.iter().map(row_to_fact).collect();
    Ok(QueryResult { facts })
}

/// `memory relation <SUBJECT> <PREDICATE>`: current state of all triples
/// matching `(subject, predicate)`.
pub async fn relation(subject: &str, predicate: &str) -> Result<QueryResult> {
    let pool = open().await?;
    let rows = sqlx::query(&format!(
        "SELECT id, subject, predicate, object, confidence, source, created_at \
         FROM {VIEW_CURRENT} WHERE subject = ?1 AND predicate = ?2 \
         ORDER BY created_at DESC"
    ))
    .bind(subject)
    .bind(predicate)
    .fetch_all(&pool)
    .await?;
    let facts = rows.iter().map(row_to_fact).collect();
    Ok(QueryResult { facts })
}

/// `memory list [--limit N]`: all current-state rows, capped at N.
pub async fn list(limit: Option<usize>) -> Result<QueryResult> {
    let pool = open().await?;
    let cap = limit.unwrap_or(LIST_DEFAULT_LIMIT).min(LIST_DEFAULT_LIMIT) as i64;
    let rows = sqlx::query(&format!(
        "SELECT id, subject, predicate, object, confidence, source, created_at \
         FROM {VIEW_CURRENT} ORDER BY created_at DESC LIMIT ?1"
    ))
    .bind(cap)
    .fetch_all(&pool)
    .await?;
    let facts = rows.iter().map(row_to_fact).collect();
    Ok(QueryResult { facts })
}

// ============ delete ============

/// `memory delete <S> <P> <O>`: soft-delete the current-state row of this
/// triple. Errors if no current row exists or it is already deleted.
///
/// Per [K001](../../../docs/adr/K001-memory-module.md): the delete targets
/// the row with `MAX(created_at) WHERE deleted_at IS NULL AND subject=? AND
/// predicate=? AND object=?`. Subsequent deletes against the same
/// already-deleted triple return `InvalidArgument("already deleted")`.
/// Deleting a triple that has no current state at all returns
/// `InvalidArgument("triple not found or already deleted")`.
pub async fn delete(subject: &str, predicate: &str, object: &str) -> Result<DeleteResult> {
    let pool = open().await?;
    let now = chrono::Utc::now().to_rfc3339();

    // Locate the current-state row for the triple.
    let row = sqlx::query(&format!(
        "SELECT id FROM {VIEW_CURRENT} WHERE subject = ?1 AND predicate = ?2 AND object = ?3 LIMIT 1"
    ))
    .bind(subject)
    .bind(predicate)
    .bind(object)
    .fetch_optional(&pool)
    .await?;

    let id: String = match row {
        Some(r) => r.get("id"),
        None => {
            return Err(AgentError::InvalidArgument(format!(
                "triple not found or already deleted: ({subject}, {predicate}, {object})"
            )));
        }
    };

    sqlx::query(&format!(
        "UPDATE {TABLE} SET deleted_at = ?1 WHERE id = ?2 AND deleted_at IS NULL"
    ))
    .bind(&now)
    .bind(&id)
    .execute(&pool)
    .await?;

    Ok(DeleteResult {
        id,
        subject: subject.to_string(),
        predicate: predicate.to_string(),
        object: object.to_string(),
        deleted_at: now,
    })
}

// ============ graph ============

/// `memory graph <SUBJECT> [--depth N] [--include-deleted]`: forward BFS
/// from `subject`. Returns the tree rooted at `subject`.
pub async fn graph(subject: &str, depth: Option<u8>, include_deleted: bool) -> Result<GraphNode> {
    let d = depth.unwrap_or(GRAPH_DEFAULT_DEPTH);
    if !(1..=GRAPH_MAX_DEPTH).contains(&d) {
        return Err(AgentError::InvalidArgument(format!(
            "--depth must be in 1..={GRAPH_MAX_DEPTH}, got {d}"
        )));
    }
    let pool = open().await?;
    let mut visited: HashSet<(String, String, String)> = HashSet::new();
    build_node(&pool, subject, d, include_deleted, &mut visited, 0).await
}

#[allow(clippy::too_many_arguments)]
fn build_node<'a>(
    pool: &'a SqlitePool,
    subject: &'a str,
    max_depth: u8,
    include_deleted: bool,
    visited: &'a mut HashSet<(String, String, String)>,
    cur_depth: u8,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<GraphNode>> + Send + 'a>> {
    Box::pin(async move {
        // Per K002, fetch direct edges for `subject`. We use a column-aware
        // query; deleted filter is built into the view when not included.
        let rows = if include_deleted {
            // Pull the latest version per (subject, predicate, object) — but
            // include deleted rows if the LATEST is deleted. This matches
            // "what is current OR was current-and-now-deleted".
            sqlx::query(
                "SELECT predicate, object FROM ( \
                    SELECT predicate, object, deleted_at, \
                        ROW_NUMBER() OVER (PARTITION BY subject, predicate, object ORDER BY created_at DESC) AS rn \
                    FROM memory WHERE subject = ?1 \
                 ) WHERE rn = 1 \
                 ORDER BY created_at DESC",
            )
            .bind(subject)
            .fetch_all(pool)
            .await?
        } else {
            sqlx::query(&format!(
                "SELECT predicate, object FROM {VIEW_CURRENT} WHERE subject = ?1 \
                 ORDER BY created_at DESC"
            ))
            .bind(subject)
            .fetch_all(pool)
            .await?
        };

        // Group by predicate.
        let mut groups: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        for r in &rows {
            let pred: String = r.get("predicate");
            let obj: String = r.get("object");
            // Mark edge visited; if already seen, skip (cycle defense).
            if !visited.insert((subject.to_string(), pred.clone(), obj.clone())) {
                continue;
            }
            groups.entry(pred).or_default().push(obj);
        }

        let mut edges: Vec<GraphEdge> = Vec::new();
        for (pred, objs) in groups {
            let mut graph_objs: Vec<GraphObject> = Vec::new();
            for obj in objs {
                // Only recurse when below max_depth and the recursion target
                // wouldn't trivially loop on the same triple. Visited check
                // above prevents revisit of the same edge; for nodes we rely
                // on the per-level edge mark. Depth guard prevents runaway.
                let sub_predicates = if cur_depth + 1 < max_depth {
                    let sub = build_node(
                        pool,
                        &obj,
                        max_depth,
                        include_deleted,
                        visited,
                        cur_depth + 1,
                    )
                    .await?;
                    sub.predicates
                } else {
                    Vec::new()
                };
                graph_objs.push(GraphObject {
                    name: obj,
                    predicates: sub_predicates,
                });
            }
            edges.push(GraphEdge {
                name: pred,
                objects: graph_objs,
            });
        }

        Ok(GraphNode {
            subject: subject.to_string(),
            predicates: edges,
        })
    })
}

// ============ history ============

/// `memory history <S> <P> <O>`: full version history (including deleted
/// rows), newest first.
pub async fn history(subject: &str, predicate: &str, object: &str) -> Result<QueryResult> {
    let pool = open().await?;
    let rows = sqlx::query(
        "SELECT id, subject, predicate, object, confidence, source, created_at, deleted_at \
         FROM memory WHERE subject = ?1 AND predicate = ?2 AND object = ?3 \
         ORDER BY created_at DESC",
    )
    .bind(subject)
    .bind(predicate)
    .bind(object)
    .fetch_all(&pool)
    .await?;
    let facts = rows.iter().map(row_to_fact_with_deleted).collect();
    Ok(QueryResult { facts })
}

// ============ helpers ============

fn row_to_fact(r: &sqlx::sqlite::SqliteRow) -> MemoryFact {
    let source: Option<String> = r.get("source");
    MemoryFact {
        id: r.get("id"),
        subject: r.get("subject"),
        predicate: r.get("predicate"),
        object: r.get("object"),
        confidence: r.get("confidence"),
        source,
        created_at: r.get("created_at"),
        deleted_at: None,
    }
}

fn row_to_fact_with_deleted(r: &sqlx::sqlite::SqliteRow) -> MemoryFact {
    let source: Option<String> = r.get("source");
    let deleted_at: Option<String> = r.get("deleted_at");
    MemoryFact {
        id: r.get("id"),
        subject: r.get("subject"),
        predicate: r.get("predicate"),
        object: r.get("object"),
        confidence: r.get("confidence"),
        source,
        created_at: r.get("created_at"),
        deleted_at,
    }
}

// ============ tests ============

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::memory::store;

    /// Open a temporary memory.db for the test. Each test gets its own file
    /// so tests don't pollute each other. The global single-instance
    /// `~/.config/everyday/memory.db` is never touched.
    async fn fresh_pool() -> SqlitePool {
        let dir = std::env::temp_dir().join(format!("everyday-mem-test-{}", store::gen_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("memory.db");
        // We can't easily swap the global path; tests that call into the
        // action handlers will hit the global path. Instead, these tests
        // exercise the SQL primitives directly against a temp pool. The
        // higher-level action tests live in mod.rs.
        store::open_at(&path).await.unwrap()
    }

    fn assert_fact_eq(f: &MemoryFact, s: &str, p: &str, o: &str) {
        assert_eq!(f.subject, s);
        assert_eq!(f.predicate, p);
        assert_eq!(f.object, o);
    }

    #[tokio::test]
    async fn insert_and_select_current_state() {
        let pool = fresh_pool().await;
        let id = store::gen_id();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO memory (id, subject, predicate, object, confidence, source, created_at) \
             VALUES (?1, ?2, ?3, ?4, 1.0, NULL, ?5)",
        )
        .bind(&id)
        .bind("user")
        .bind("prefers")
        .bind("rust")
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();

        let row: (String, String, String) = sqlx::query_as(&format!(
            "SELECT id, predicate, object FROM {VIEW_CURRENT} WHERE subject = 'user'"
        ))
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, id);
        assert_eq!(row.1, "prefers");
        assert_eq!(row.2, "rust");
    }

    #[tokio::test]
    async fn append_only_versions_keep_history() {
        // Re-adding the same triple creates a new row. Both rows exist in
        // the table; only the latest survives in the view.
        let pool = fresh_pool().await;
        for _ in 0..2 {
            sqlx::query(
                "INSERT INTO memory (id, subject, predicate, object, confidence, source, created_at) \
                 VALUES (?1, ?2, ?3, ?4, 1.0, NULL, ?5)",
            )
            .bind(store::gen_id())
            .bind("user")
            .bind("prefers")
            .bind("rust")
            .bind(chrono::Utc::now().to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();
        }

        // View: only the latest (one row, since both inserts target the
        // same triple partition).
        let n: (i64,) = sqlx::query_as(&format!(
            "SELECT COUNT(*) FROM {VIEW_CURRENT} WHERE subject = 'user'"
        ))
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(n.0, 1);

        // Table: both rows.
        let n: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM memory WHERE subject = 'user'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n.0, 2);
    }

    #[tokio::test]
    async fn soft_delete_hides_from_view() {
        let pool = fresh_pool().await;
        let id = store::gen_id();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO memory (id, subject, predicate, object, confidence, source, created_at) \
             VALUES (?1, 'user', 'prefers', 'rust', 1.0, NULL, ?2)",
        )
        .bind(&id)
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query("UPDATE memory SET deleted_at = ?1 WHERE id = ?2")
            .bind(&now)
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();

        let n: (i64,) = sqlx::query_as(&format!(
            "SELECT COUNT(*) FROM {VIEW_CURRENT} WHERE subject = 'user'"
        ))
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(n.0, 0);

        // Table still has the row.
        let n: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM memory")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n.0, 1);
    }

    #[test]
    fn graph_node_serializes_nested() {
        // Smoke test: ensure Serialize shapes match the ADR description.
        let node = GraphNode {
            subject: "user".to_string(),
            predicates: vec![GraphEdge {
                name: "prefers".to_string(),
                objects: vec![GraphObject {
                    name: "rust".to_string(),
                    predicates: vec![],
                }],
            }],
        };
        let v = serde_json::to_value(&node).unwrap();
        assert_eq!(v["subject"], "user");
        assert_eq!(v["predicates"][0]["name"], "prefers");
        assert_eq!(v["predicates"][0]["objects"][0]["name"], "rust");
        // Suppress unused warnings on assert_fact_eq.
        let _ = assert_fact_eq;
    }
}
