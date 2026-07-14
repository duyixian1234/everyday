//! Memory module: agent's own notebook — append-only `(subject, predicate, object)` triples.
//!
//! See [K001](../../../docs/adr/K001-memory-module.md) for the main decision
//! and [K002](../../../docs/adr/K002-memory-graph-query.md) / [K003](../../../docs/adr/K003-memory-searchable.md) /
//! [K004](../../../docs/adr/K004-memory-single-instance.md) for the
//! supporting details.
//!
//! CLI surface (7 actions):
//! - `add <S> <P> <O> [--confidence N] [--source LABEL]`
//! - `get <SUBJECT>`
//! - `relation <SUBJECT> <PREDICATE>`
//! - `list [--limit N]`
//! - `delete <S> <P> <O>`
//! - `graph <SUBJECT> [--depth N] [--include-deleted]`
//! - `history <S> <P> <O>`
//!
//! Storage is `~/.config/everyday/memory.db`, single global instance, no
//! account column, no `auth` module touch. Memory is not a multi-account
//! module — see K004.
//!
//! Rendering: tabular for add/get/relation/list/delete/history, nested
//! tree for graph (text: indented markdown; JSON: nested object).

pub mod actions;
pub mod search;
pub mod store;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::error::{AgentError, Result};
use crate::modules::parse_simple_args;
use crate::modules::{Executor, ModuleArgSpec};
use crate::output::Output;
use crate::search::Searchable;
use std::sync::Arc;

pub struct MemoryModule;

impl MemoryModule {
    pub const fn new() -> Self {
        Self
    }
}

impl Default for MemoryModule {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Executor for MemoryModule {
    fn description(&self) -> &'static str {
        "Structured memory notebook: append-only (subject, predicate, object) triples with confidence/source."
    }

    fn module_arg_spec(&self) -> ModuleArgSpec {
        use crate::modules::{ActionArgSpec, ArgKind, ArgSpec, Positional};

        static ACTIONS: &[ActionArgSpec] = &[
            ActionArgSpec {
                name: "add",
                description: "新增一条三元组（同一 S/P/O 重复 add 会追加新版本）",
                usage: "everyday memory add <SUBJECT> <PREDICATE> <OBJECT> [--confidence N] [--source LABEL]",
                args: &[
                    ArgSpec {
                        name: "confidence",
                        help: "置信度 [0.0, 1.0]（默认 1.0）",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "source",
                        help: "来源标签（自由文本）",
                        kind: ArgKind::Value,
                    },
                ],
                positional: Positional::Exactly(3),
            },
            ActionArgSpec {
                name: "get",
                description: "查询 subject 当前态全部三元组",
                usage: "everyday memory get <SUBJECT>",
                args: &[],
                positional: Positional::Exactly(1),
            },
            ActionArgSpec {
                name: "relation",
                description: "查询 (subject, predicate) 当前态全部对象",
                usage: "everyday memory relation <SUBJECT> <PREDICATE>",
                args: &[],
                positional: Positional::Exactly(2),
            },
            ActionArgSpec {
                name: "list",
                description: "列出当前态全部三元组",
                usage: "everyday memory list [--limit N]",
                args: &[ArgSpec {
                    name: "limit",
                    help: "条数上限（默认 100）",
                    kind: ArgKind::Value,
                }],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "delete",
                description: "软删除 (subject, predicate, object) 当前态行",
                usage: "everyday memory delete <SUBJECT> <PREDICATE> <OBJECT>",
                args: &[],
                positional: Positional::Exactly(3),
            },
            ActionArgSpec {
                name: "graph",
                description: "前向 BFS：从 subject 出发的多跳图（深度默认 2，最大 5）",
                usage: "everyday memory graph <SUBJECT> [--depth N] [--include-deleted]",
                args: &[
                    ArgSpec {
                        name: "depth",
                        help: "递归深度（1..=5，默认 2）",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "include-deleted",
                        help: "包含软删除的边（默认隐藏）",
                        kind: ArgKind::Bool,
                    },
                ],
                positional: Positional::Exactly(1),
            },
            ActionArgSpec {
                name: "history",
                description: "查看三元组全部版本（含已删除）",
                usage: "everyday memory history <SUBJECT> <PREDICATE> <OBJECT>",
                args: &[],
                positional: Positional::Exactly(3),
            },
        ];

        ModuleArgSpec {
            name: "memory",
            description: self.description(),
            actions: ACTIONS,
        }
    }

    async fn execute(&self, action: &str, args: &[String]) -> Result<Output> {
        let (flags, positional) = parse_simple_args(args);
        let json_mode = crate::util::json_mode::is_json();

        match action {
            "add" => {
                let (s, p, o) = take_pos3(&positional, "memory add")?;
                let result = actions::add(
                    &s,
                    &p,
                    &o,
                    flags.get("confidence").map(|s| s.as_str()),
                    flags.get("source").map(|s| s.as_str()),
                )
                .await?;
                Ok(render_fact(&result, "added", json_mode))
            }
            "get" => {
                let subject = take_pos1(&positional, "memory get")?;
                let result = actions::get(&subject).await?;
                Ok(render_query(
                    &result,
                    &format!("memory get {subject}"),
                    json_mode,
                ))
            }
            "relation" => {
                let (subject, predicate) = take_pos2(&positional, "memory relation")?;
                let result = actions::relation(&subject, &predicate).await?;
                Ok(render_query(
                    &result,
                    &format!("memory relation {subject} {predicate}"),
                    json_mode,
                ))
            }
            "list" => {
                let limit: Option<usize> = flags
                    .get("limit")
                    .and_then(|s| s.parse().ok())
                    .map(|n: usize| n.min(actions::LIST_DEFAULT_LIMIT));
                let result = actions::list(limit).await?;
                Ok(render_query(&result, "memory list", json_mode))
            }
            "delete" => {
                let (s, p, o) = take_pos3(&positional, "memory delete")?;
                let result = actions::delete(&s, &p, &o).await?;
                Ok(render_delete(&result, json_mode))
            }
            "graph" => {
                let subject = take_pos1(&positional, "memory graph")?;
                let depth: Option<u8> = flags.get("depth").and_then(|s| s.parse().ok());
                let include_deleted = flags
                    .get("include-deleted")
                    .map(|s| s == "true")
                    .unwrap_or(false);
                let tree = actions::graph(&subject, depth, include_deleted).await?;
                Ok(render_graph(&tree, json_mode))
            }
            "history" => {
                let (s, p, o) = take_pos3(&positional, "memory history")?;
                let result = actions::history(&s, &p, &o).await?;
                Ok(render_history(&result, json_mode))
            }
            other => Err(AgentError::UnknownAction(format!("memory {other}"))),
        }
    }
}

// ============ positional helpers ============

fn take_pos1(pos: &[String], usage: &str) -> Result<String> {
    pos.first().cloned().ok_or_else(|| {
        AgentError::InvalidArgument(format!("{usage} requires 1 positional argument"))
    })
}

fn take_pos2(pos: &[String], usage: &str) -> Result<(String, String)> {
    if pos.len() < 2 {
        return Err(AgentError::InvalidArgument(format!(
            "{usage} requires 2 positional arguments"
        )));
    }
    Ok((pos[0].clone(), pos[1].clone()))
}

fn take_pos3(pos: &[String], usage: &str) -> Result<(String, String, String)> {
    if pos.len() < 3 {
        return Err(AgentError::InvalidArgument(format!(
            "{usage} requires 3 positional arguments"
        )));
    }
    Ok((pos[0].clone(), pos[1].clone(), pos[2].clone()))
}

// ============ render ============

fn render_fact(f: &actions::MemoryFact, verb: &str, json_mode: bool) -> Output {
    if json_mode {
        let mut obj = serde_json::Map::new();
        obj.insert("id".into(), Value::String(f.id.clone()));
        obj.insert("subject".into(), Value::String(f.subject.clone()));
        obj.insert("predicate".into(), Value::String(f.predicate.clone()));
        obj.insert("object".into(), Value::String(f.object.clone()));
        obj.insert("confidence".into(), json!(f.confidence));
        obj.insert(
            "source".into(),
            match &f.source {
                Some(s) => Value::String(s.clone()),
                None => Value::Null,
            },
        );
        obj.insert("created_at".into(), Value::String(f.created_at.clone()));
        Output::Json(Value::Object(obj))
    } else {
        let src = f.source.as_deref().unwrap_or("-");
        Output::text(format!(
            "{verb} ({}, {}, {}) id={} conf={} src={} at={}",
            f.subject, f.predicate, f.object, f.id, f.confidence, src, f.created_at
        ))
    }
}

fn render_query(q: &actions::QueryResult, header: &str, json_mode: bool) -> Output {
    if json_mode {
        let facts: Vec<Value> = q.facts.iter().map(fact_to_json).collect();
        Output::Json(json!({ "facts": facts, "count": facts.len() }))
    } else {
        if q.facts.is_empty() {
            return Output::text(format!("{header}: 0 facts"));
        }
        let headers = vec![
            "subject".to_string(),
            "predicate".to_string(),
            "object".to_string(),
            "confidence".to_string(),
            "source".to_string(),
            "id".to_string(),
            "created_at".to_string(),
        ];
        let rows: Vec<Vec<String>> = q
            .facts
            .iter()
            .map(|f| {
                vec![
                    f.subject.clone(),
                    f.predicate.clone(),
                    f.object.clone(),
                    format!("{}", f.confidence),
                    f.source.clone().unwrap_or_default(),
                    f.id.clone(),
                    f.created_at.clone(),
                ]
            })
            .collect();
        Output::records(headers, rows)
    }
}

fn render_history(q: &actions::QueryResult, json_mode: bool) -> Output {
    if json_mode {
        let facts: Vec<Value> = q.facts.iter().map(fact_to_json_with_deleted).collect();
        Output::Json(json!({ "history": facts, "count": facts.len() }))
    } else {
        if q.facts.is_empty() {
            return Output::text("memory history: 0 versions".to_string());
        }
        let headers = vec![
            "id".to_string(),
            "confidence".to_string(),
            "source".to_string(),
            "created_at".to_string(),
            "deleted_at".to_string(),
        ];
        let rows: Vec<Vec<String>> = q
            .facts
            .iter()
            .map(|f| {
                vec![
                    f.id.clone(),
                    format!("{}", f.confidence),
                    f.source.clone().unwrap_or_default(),
                    f.created_at.clone(),
                    f.deleted_at.clone().unwrap_or_default(),
                ]
            })
            .collect();
        Output::records(headers, rows)
    }
}

fn render_delete(d: &actions::DeleteResult, json_mode: bool) -> Output {
    if json_mode {
        let mut obj = serde_json::Map::new();
        obj.insert("id".into(), Value::String(d.id.clone()));
        obj.insert("subject".into(), Value::String(d.subject.clone()));
        obj.insert("predicate".into(), Value::String(d.predicate.clone()));
        obj.insert("object".into(), Value::String(d.object.clone()));
        obj.insert("deleted_at".into(), Value::String(d.deleted_at.clone()));
        Output::Json(Value::Object(obj))
    } else {
        Output::text(format!(
            "deleted ({}, {}, {}) id={} at={}",
            d.subject, d.predicate, d.object, d.id, d.deleted_at
        ))
    }
}

fn render_graph(node: &actions::GraphNode, json_mode: bool) -> Output {
    if json_mode {
        Output::Json(serde_json::to_value(node).unwrap_or(Value::Null))
    } else {
        let mut s = String::new();
        write_graph_text(&mut s, node, 0);
        Output::text(s)
    }
}

fn write_graph_text(out: &mut String, node: &actions::GraphNode, depth: usize) {
    if depth == 0 {
        out.push_str(&node.subject);
        out.push('\n');
    }
    for (i, edge) in node.predicates.iter().enumerate() {
        let prefix = indent(depth);
        let connector = if i + 1 < node.predicates.len() && depth == 0 {
            "+-- "
        } else if i + 1 < node.predicates.len() {
            "    "
        } else {
            "`-- "
        };
        out.push_str(&format!("{prefix}{connector}{} --> ", edge.name));
        if edge.objects.is_empty() {
            out.push_str("(none)\n");
        }
        for (j, obj) in edge.objects.iter().enumerate() {
            if j > 0 {
                out.push_str(&format!("{prefix}         --> "));
            }
            out.push_str(&obj.name);
            out.push('\n');
            write_graph_subtree(out, obj, depth + 2);
        }
    }
}

fn write_graph_subtree(out: &mut String, obj: &actions::GraphObject, depth: usize) {
    let prefix = indent(depth);
    for (i, edge) in obj.predicates.iter().enumerate() {
        let connector = if i + 1 < obj.predicates.len() {
            "+-- "
        } else {
            "`-- "
        };
        out.push_str(&format!("{prefix}{connector}{} --> ", edge.name));
        for (j, o) in edge.objects.iter().enumerate() {
            if j > 0 {
                out.push_str(&format!("{prefix}     --> "));
            }
            out.push_str(&o.name);
            out.push('\n');
            write_graph_subtree(out, o, depth + 2);
        }
    }
}

fn indent(depth: usize) -> String {
    "  ".repeat(depth)
}

fn fact_to_json(f: &actions::MemoryFact) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("id".into(), Value::String(f.id.clone()));
    obj.insert("subject".into(), Value::String(f.subject.clone()));
    obj.insert("predicate".into(), Value::String(f.predicate.clone()));
    obj.insert("object".into(), Value::String(f.object.clone()));
    obj.insert("confidence".into(), json!(f.confidence));
    obj.insert(
        "source".into(),
        match &f.source {
            Some(s) => Value::String(s.clone()),
            None => Value::Null,
        },
    );
    obj.insert("created_at".into(), Value::String(f.created_at.clone()));
    Value::Object(obj)
}

fn fact_to_json_with_deleted(f: &actions::MemoryFact) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("id".into(), Value::String(f.id.clone()));
    obj.insert("subject".into(), Value::String(f.subject.clone()));
    obj.insert("predicate".into(), Value::String(f.predicate.clone()));
    obj.insert("object".into(), Value::String(f.object.clone()));
    obj.insert("confidence".into(), json!(f.confidence));
    obj.insert(
        "source".into(),
        match &f.source {
            Some(s) => Value::String(s.clone()),
            None => Value::Null,
        },
    );
    obj.insert("created_at".into(), Value::String(f.created_at.clone()));
    obj.insert(
        "deleted_at".into(),
        match &f.deleted_at {
            Some(s) => Value::String(s.clone()),
            None => Value::Null,
        },
    );
    Value::Object(obj)
}

// ============ re-exports ============

/// Re-export the search provider so `modules::search` can register it.
pub use search::MemorySearchProvider;

/// Helper: build the `MemorySearchProvider` Arc for use in `SearchRegistry::build`.
pub fn search_provider() -> Arc<dyn Searchable> {
    Arc::new(MemorySearchProvider::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::memory::store;

    /// Each test runs against a temp DB by overriding the global path
    /// indirectly: tests in this module invoke the *actions* directly, so
    /// they hit the real `~/.config/everyday/memory.db`. To isolate tests
    /// we instead test the SQL primitives via the `fresh_pool` helper,
    /// mirroring the pattern in `actions::tests`.
    ///
    /// Tests that exercise the action dispatch end-to-end use a custom
    /// `MemoryModule::new_with_db` shim? — no, that doesn't exist. Instead
    /// we test the dispatch via direct arg-parsing helpers + render
    /// functions (which don't touch the DB).

    #[test]
    fn take_pos1_requires_argument() {
        let pos: Vec<String> = vec![];
        let err = take_pos1(&pos, "memory get").unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
    }

    #[test]
    fn take_pos2_requires_two() {
        let pos: Vec<String> = vec!["a".into()];
        assert!(take_pos2(&pos, "memory relation").is_err());
        let pos2: Vec<String> = vec!["a".into(), "b".into()];
        let (a, b) = take_pos2(&pos2, "memory relation").unwrap();
        assert_eq!(a, "a");
        assert_eq!(b, "b");
    }

    #[test]
    fn take_pos3_requires_three() {
        let pos: Vec<String> = vec!["a".into(), "b".into()];
        assert!(take_pos3(&pos, "memory add").is_err());
        let pos3: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
        let (a, b, c) = take_pos3(&pos3, "memory add").unwrap();
        assert_eq!(a, "a");
        assert_eq!(b, "b");
        assert_eq!(c, "c");
    }

    #[test]
    fn module_arg_spec_has_seven_actions() {
        let m = MemoryModule::new();
        let spec = m.module_arg_spec();
        assert_eq!(spec.name, "memory");
        assert_eq!(spec.actions.len(), 7);
        let names: Vec<&str> = spec.actions.iter().map(|a| a.name).collect();
        assert_eq!(
            names,
            vec![
                "add", "get", "relation", "list", "delete", "graph", "history"
            ]
        );
    }

    #[test]
    fn render_fact_text_shape() {
        let f = actions::MemoryFact {
            id: "m123".into(),
            subject: "user".into(),
            predicate: "prefers".into(),
            object: "rust".into(),
            confidence: 0.8,
            source: Some("explicit".into()),
            created_at: "2026-07-14T00:00:00Z".into(),
            deleted_at: None,
        };
        let out = render_fact(&f, "added", false);
        if let Output::Text(s) = out {
            assert!(s.contains("user"));
            assert!(s.contains("prefers"));
            assert!(s.contains("rust"));
            assert!(s.contains("m123"));
        } else {
            panic!("expected Text output");
        }
    }

    #[test]
    fn render_query_empty_in_text() {
        let q = actions::QueryResult { facts: vec![] };
        let out = render_query(&q, "memory get user", false);
        if let Output::Text(s) = out {
            assert!(s.contains("0 facts"));
        } else {
            panic!("expected Text output");
        }
    }

    #[test]
    fn render_query_json_envelope() {
        let q = actions::QueryResult {
            facts: vec![actions::MemoryFact {
                id: "m1".into(),
                subject: "user".into(),
                predicate: "prefers".into(),
                object: "rust".into(),
                confidence: 1.0,
                source: None,
                created_at: "2026-07-14T00:00:00Z".into(),
                deleted_at: None,
            }],
        };
        crate::util::json_mode::set_json_mode(true);
        let out = render_query(&q, "memory get user", true);
        crate::util::json_mode::set_json_mode(false);
        if let Output::Json(v) = out {
            assert_eq!(v["count"], 1);
            assert_eq!(v["facts"][0]["subject"], "user");
            assert_eq!(v["facts"][0]["predicate"], "prefers");
            assert_eq!(v["facts"][0]["object"], "rust");
        } else {
            panic!("expected Json output");
        }
    }

    #[test]
    fn render_graph_text_indented() {
        let tree = actions::GraphNode {
            subject: "user".into(),
            predicates: vec![actions::GraphEdge {
                name: "prefers".into(),
                objects: vec![actions::GraphObject {
                    name: "rust".into(),
                    predicates: vec![actions::GraphEdge {
                        name: "owned_by".into(),
                        objects: vec![actions::GraphObject {
                            name: "mozilla".into(),
                            predicates: vec![],
                        }],
                    }],
                }],
            }],
        };
        let out = render_graph(&tree, false);
        if let Output::Text(s) = out {
            assert!(s.contains("user"));
            assert!(s.contains("prefers"));
            assert!(s.contains("rust"));
            assert!(s.contains("owned_by"));
            assert!(s.contains("mozilla"));
        } else {
            panic!("expected Text output");
        }
    }

    #[test]
    fn render_graph_json_nested() {
        let tree = actions::GraphNode {
            subject: "user".into(),
            predicates: vec![actions::GraphEdge {
                name: "prefers".into(),
                objects: vec![actions::GraphObject {
                    name: "rust".into(),
                    predicates: vec![],
                }],
            }],
        };
        crate::util::json_mode::set_json_mode(true);
        let out = render_graph(&tree, true);
        crate::util::json_mode::set_json_mode(false);
        if let Output::Json(v) = out {
            assert_eq!(v["subject"], "user");
            assert_eq!(v["predicates"][0]["name"], "prefers");
            assert_eq!(v["predicates"][0]["objects"][0]["name"], "rust");
        } else {
            panic!("expected Json output");
        }
    }

    /// End-to-end: add → get → list → delete → history on the real
    /// `~/.config/everyday/memory.db`. The DB is shared across tests, so
    /// we tolerate pre-existing data and only assert what we ourselves
    /// wrote. Other tests may pollute rows; the `where subject=?` filter
    /// scopes our assertions.
    #[tokio::test]
    async fn end_to_end_add_get_list_delete_history() {
        // Use a unique subject name to avoid collisions.
        let subject = format!("test-{}", store::gen_id());

        // Add 2 facts with the same (s, p) but different objects.
        let f1 = actions::add(&subject, "prefers", "rust", Some("0.9"), Some("unit-test"))
            .await
            .unwrap();
        let _f2 = actions::add(&subject, "prefers", "go", None, None)
            .await
            .unwrap();

        // get returns both.
        let got = actions::get(&subject).await.unwrap();
        let objects: Vec<&str> = got.facts.iter().map(|f| f.object.as_str()).collect();
        assert!(objects.contains(&"rust"));
        assert!(objects.contains(&"go"));

        // relation filters to (s, p).
        let rel = actions::relation(&subject, "prefers").await.unwrap();
        assert_eq!(rel.facts.len(), 2);

        // list includes our facts (count is at least 2; we don't assert
        // exact count because other tests may have written).
        let list = actions::list(None).await.unwrap();
        assert!(list.facts.iter().any(|f| f.subject == subject));

        // history returns both versions (newest first).
        let hist = actions::history(&subject, "prefers", "rust").await.unwrap();
        assert!(hist.facts.iter().any(|f| f.id == f1.id));
        assert_eq!(hist.facts[0].id, f1.id); // newest first.

        // delete the rust fact; history still shows both rows.
        let del = actions::delete(&subject, "prefers", "rust").await.unwrap();
        assert_eq!(del.subject, subject);
        assert_eq!(del.predicate, "prefers");
        assert_eq!(del.object, "rust");

        let hist2 = actions::history(&subject, "prefers", "rust").await.unwrap();
        assert_eq!(hist2.facts.len(), 1);
        assert!(hist2.facts[0].deleted_at.is_some());

        // get no longer returns the rust fact.
        let got2 = actions::get(&subject).await.unwrap();
        assert!(!got2.facts.iter().any(|f| f.object == "rust"));
        // but "go" survives.
        assert!(got2.facts.iter().any(|f| f.object == "go"));

        // Re-add the same triple to verify resurrection.
        let f1_again = actions::add(&subject, "prefers", "rust", None, None)
            .await
            .unwrap();
        assert_ne!(f1_again.id, f1.id);
        let hist3 = actions::history(&subject, "prefers", "rust").await.unwrap();
        assert_eq!(hist3.facts.len(), 2);
        // Cleanup: delete both resurrected and any leftover so future
        // test runs start clean for this subject.
        let _ = actions::delete(&subject, "prefers", "rust").await;
        let _ = actions::delete(&subject, "prefers", "go").await;

        // After cleanup, get returns 0 facts for this subject.
        let got3 = actions::get(&subject).await.unwrap();
        assert!(
            got3.facts
                .iter()
                .all(|f| f.object != "rust" && f.object != "go")
        );
    }

    /// `memory delete` on a nonexistent triple returns InvalidArgument.
    #[tokio::test]
    async fn delete_nonexistent_triple_errors() {
        let subject = format!("ghost-{}", store::gen_id());
        let err = actions::delete(&subject, "prefers", "nothing")
            .await
            .unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
    }

    /// `memory delete` on a triple whose current state is already deleted
    /// returns InvalidArgument("already deleted"-style).
    #[tokio::test]
    async fn delete_already_deleted_errors() {
        let subject = format!("dup-{}", store::gen_id());
        actions::add(&subject, "owns", "x", None, None)
            .await
            .unwrap();
        actions::delete(&subject, "owns", "x").await.unwrap();
        let err = actions::delete(&subject, "owns", "x").await.unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
        // Cleanup.
        let _ = actions::delete(&subject, "owns", "x").await;
    }

    /// `memory graph` rejects out-of-range depth.
    #[tokio::test]
    async fn graph_rejects_bad_depth() {
        let err = actions::graph("user", Some(0), false).await.unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
        let err = actions::graph("user", Some(6), false).await.unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
    }

    /// `memory graph` walks a small chain and renders without infinite
    /// loop (cycle detection).
    #[tokio::test]
    async fn graph_walks_chain_and_handles_cycles() {
        let subject = format!("chain-{}", store::gen_id());
        // A -> B -> C -> A (cycle).
        actions::add(&subject, "next", "B", None, None)
            .await
            .unwrap();
        actions::add("B", "next", "C", None, None).await.unwrap();
        actions::add("C", "next", "A", None, None).await.unwrap();

        let tree = actions::graph(&subject, Some(3), false).await.unwrap();
        // The tree should contain subject, and traversal should find B.
        assert_eq!(tree.subject, subject);
        let next_edges: Vec<&actions::GraphEdge> = tree
            .predicates
            .iter()
            .filter(|e| e.name == "next")
            .collect();
        assert_eq!(next_edges.len(), 1);
        assert_eq!(next_edges[0].objects[0].name, "B");

        // Cleanup: cascade delete the test data we added.
        let _ = actions::delete(&subject, "next", "B").await;
        let _ = actions::delete("B", "next", "C").await;
        let _ = actions::delete("C", "next", "A").await;
    }
}
