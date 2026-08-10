//! Leveled logging via `tracing`, writing to stderr with the project's
//! text/JSON output contract preserved (R001):
//!
//! - text mode: compact lines (`[req] module action ok in 12ms`).
//! - `--json` mode: structured `{"_log": ...}` lines with the exact field
//!   sets the old middleware emitted (start/ok/error/error_detail).
//!
//! Warning-shaped events (`warning: ...` / `{"_warning": ...}`) do NOT pass
//! through this layer — they are emitted by their call sites directly (see
//! the warning-site migration, follow-up ticket T2).
//!
//! Only events with an `everyday` target are rendered; dependency crates
//! (rmcp, hyper, …) are deliberately not surfaced, matching the pre-tracing
//! behavior where nothing was set up for them.
//!
//! Level mapping (`-v` count, set once at startup by `main.rs`):
//! 0 → WARN (default: warnings/errors visible), 1 → INFO, ≥2 → DEBUG.

use std::fmt;

#[cfg(test)]
use std::sync::{Arc, Mutex};

use serde_json::json;
use tracing::field::{Field, Visit};
use tracing::level_filters::LevelFilter;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry;

/// Level for a `-v` count: 0 → WARN, 1 → INFO, ≥2 → DEBUG.
pub(crate) fn level_for_verbose(verbose: u8) -> LevelFilter {
    match verbose {
        0 => LevelFilter::WARN,
        1 => LevelFilter::INFO,
        _ => LevelFilter::DEBUG,
    }
}

/// Install the process-wide subscriber. Idempotent: a second call is a
/// harmless no-op (only `main` calls this, once).
pub fn init(verbose: u8, json: bool) {
    let layer = EverydayLayer::new(json).with_filter(level_for_verbose(verbose));
    let subscriber = registry().with(layer);
    let _ = tracing::subscriber::set_global_default(subscriber);
}

/// Custom layer: renders only `everyday`-targeted events, in JSON or text
/// form depending on the process's render mode (captured at construction —
/// one command per process, so the mode is fixed for the lifetime).
#[derive(Debug, Clone)]
pub struct EverydayLayer {
    json: bool,
    sink: Sink,
}

#[derive(Debug, Clone)]
enum Sink {
    Stderr,
    #[cfg(test)]
    Buf(Arc<Mutex<Vec<u8>>>),
}

impl Sink {
    fn write_line(&self, line: &str) {
        match self {
            Sink::Stderr => eprintln!("{line}"),
            #[cfg(test)]
            Sink::Buf(buf) => {
                let mut v = buf.lock().unwrap();
                v.extend_from_slice(line.as_bytes());
                v.push(b'\n');
            }
        }
    }
}

impl EverydayLayer {
    pub fn new(json: bool) -> Self {
        Self {
            json,
            sink: Sink::Stderr,
        }
    }

    #[cfg(test)]
    fn with_buf(json: bool, buf: Arc<Mutex<Vec<u8>>>) -> Self {
        Self {
            json,
            sink: Sink::Buf(buf),
        }
    }
}

impl<S> Layer<S> for EverydayLayer
where
    S: Subscriber + for<'a> registry::LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        // Only our own events render; dependency-crate diagnostics are out of
        // scope (they were never surfaced before tracing). Exact match (or
        // `everyday::` sub-targets) — never a bare prefix, so a hypothetical
        // `everyday-core` dependency cannot leak through.
        let target = event.metadata().target();
        if target != "everyday" && !target.starts_with("everyday::") {
            return;
        }
        if self.json {
            let mut map = serde_json::Map::new();
            event.record(&mut JsonVisitor(&mut map));
            if map.is_empty() {
                return;
            }
            let value = serde_json::Value::Object(map);
            if let Ok(line) = serde_json::to_string(&value) {
                self.sink.write_line(&line);
            }
        } else if let Some(line) = render_text(event) {
            self.sink.write_line(&line);
        }
    }
}

/// Collect an event's fields into a JSON object, preserving the R001
/// `{"_log" / "_warning": ...}` shapes. Events are emitted field-only (no
/// implicit `message`), so every recorded field maps 1:1 to a JSON key.
struct JsonVisitor<'a>(&'a mut serde_json::Map<String, serde_json::Value>);

impl<'a> Visit for JsonVisitor<'a> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_string(), json!(value));
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(
            field.name().to_string(),
            serde_json::Value::Number(value.into()),
        );
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(
            field.name().to_string(),
            serde_json::Value::Number(value.into()),
        );
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name().to_string(), json!(value));
    }
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        // `%field` (Display) events arrive here via a Debug wrapper whose
        // Debug impl delegates to Display, so the string is unquoted.
        self.0
            .insert(field.name().to_string(), json!(format!("{value:?}")));
    }
}

/// Render the text-mode line for the middleware events (`_log` = start/ok/
/// error/error_detail), matching the pre-tracing compact format. Returns
/// `None` for everyday events without a `_log` kind — callers must not emit
/// a blank line for them.
fn render_text(event: &Event<'_>) -> Option<String> {
    let mut f = TextFields::default();
    event.record(&mut f);
    let kind = f._log.as_deref()?;
    let rid = f.request_id.as_deref().unwrap_or("");
    let module = f.module.as_deref().unwrap_or("");
    let action = f.action.as_deref().unwrap_or("");
    let line = match kind {
        "start" => format!("[{rid}] {module} {action} start"),
        "ok" => format!(
            "[{rid}] {module} {action} ok in {}ms",
            f.elapsed_ms.unwrap_or(0)
        ),
        "error" => format!(
            "[{rid}] {module} {action} error in {}ms",
            f.elapsed_ms.unwrap_or(0)
        ),
        "error_detail" => format!(
            "[{rid}] {module} {action} error: {}",
            f.message.as_deref().unwrap_or("")
        ),
        other => format!("[{rid}] {module} {action} {other}"),
    };
    Some(line)
}

#[derive(Default)]
struct TextFields {
    _log: Option<String>,
    request_id: Option<String>,
    module: Option<String>,
    action: Option<String>,
    elapsed_ms: Option<u64>,
    message: Option<String>,
}

impl Visit for TextFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "_log" => self._log = Some(value.to_string()),
            "request_id" => self.request_id = Some(value.to_string()),
            "module" => self.module = Some(value.to_string()),
            "action" => self.action = Some(value.to_string()),
            "message" => self.message = Some(value.to_string()),
            _ => {}
        }
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "elapsed_ms" {
            self.elapsed_ms = Some(value);
        }
    }
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.record_str(field, &format!("{value:?}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::subscriber::with_default;

    fn run_with<F: FnOnce()>(json: bool, level: LevelFilter, f: F) -> String {
        let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let layer = EverydayLayer::with_buf(json, buf.clone()).with_filter(level);
        with_default(registry().with(layer), f);
        String::from_utf8(buf.lock().unwrap().clone()).unwrap()
    }

    fn emit_middleware_events() {
        tracing::info!(
            target: "everyday",
            _log = "start",
            request_id = "req-1",
            caller = "cli",
            module = "mail",
            action = "list",
        );
        tracing::info!(
            target: "everyday",
            _log = "ok",
            request_id = "req-1",
            caller = "cli",
            module = "mail",
            action = "list",
            elapsed_ms = 5u64,
        );
        tracing::info!(
            target: "everyday",
            _log = "error_detail",
            request_id = "req-1",
            caller = "cli",
            module = "mail",
            action = "list",
            message = "boom",
        );
    }

    #[test]
    fn json_mode_preserves_log_shape() {
        let out = run_with(true, LevelFilter::INFO, emit_middleware_events);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3, "three events, three lines: {out}");

        let start: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(start["_log"], "start");
        assert_eq!(start["request_id"], "req-1");
        assert_eq!(start["caller"], "cli");
        assert_eq!(start["module"], "mail");
        assert_eq!(start["action"], "list");
        assert!(start.get("message").is_none(), "no implicit message key");

        let ok: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(ok["_log"], "ok");
        assert_eq!(ok["elapsed_ms"], 5);

        let err: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(err["_log"], "error_detail");
        assert_eq!(err["message"], "boom");
    }

    #[test]
    fn text_mode_preserves_compact_format() {
        let out = run_with(false, LevelFilter::INFO, emit_middleware_events);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines,
            vec![
                "[req-1] mail list start",
                "[req-1] mail list ok in 5ms",
                "[req-1] mail list error: boom",
            ]
        );
    }

    #[test]
    fn warn_level_silences_info_events() {
        let out = run_with(false, LevelFilter::WARN, emit_middleware_events);
        assert!(
            out.is_empty(),
            "default WARN must silence info events: {out}"
        );
    }

    #[test]
    fn non_everyday_targets_are_ignored() {
        let out = run_with(true, LevelFilter::INFO, || {
            tracing::info!(target: "rmcp", _log = "start", module = "x");
        });
        assert!(
            out.is_empty(),
            "dependency-crate events must not render: {out}"
        );
    }

    #[test]
    fn everyday_prefix_only_does_not_leak() {
        // A crate named `everyday-core` must not render either (exact-target
        // match, not bare prefix).
        let out = run_with(true, LevelFilter::INFO, || {
            tracing::info!(target: "everyday-core", _log = "start");
        });
        assert!(out.is_empty(), "prefix-only targets must not render: {out}");
    }

    #[test]
    fn verbose_level_mapping() {
        assert_eq!(level_for_verbose(0), LevelFilter::WARN);
        assert_eq!(level_for_verbose(1), LevelFilter::INFO);
        assert_eq!(level_for_verbose(2), LevelFilter::DEBUG);
        assert_eq!(level_for_verbose(3), LevelFilter::DEBUG);
    }

    #[test]
    fn display_fields_route_through_record_debug() {
        // Real middleware events use `%value` (Display); those arrive via
        // `record_debug`, not `record_str` — lock that path explicitly.
        let out = run_with(true, LevelFilter::INFO, || {
            let rid = String::from("req-9");
            let msg = String::from("display boom");
            tracing::info!(
                target: "everyday",
                _log = "error_detail",
                request_id = %rid,
                message = %msg,
            );
        });
        let v: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(v["request_id"], "req-9");
        assert_eq!(v["message"], "display boom");
    }

    #[test]
    fn event_without_log_kind_renders_nothing() {
        // Unknown/absent `_log` must not produce a blank stderr line.
        let out = run_with(false, LevelFilter::INFO, || {
            tracing::info!(target: "everyday", module = "x");
        });
        assert!(out.is_empty(), "no blank line expected: {out}");
    }
}
