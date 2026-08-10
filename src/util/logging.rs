//! Leveled logging via `tracing`, writing to stderr with the project's
//! text/JSON output contract preserved (R001):
//!
//! - text mode: compact lines (`[req] module action ok in 12ms`,
//!   `warning: ...`, `timeline: ...`).
//! - `--json` mode: structured `{"_log": ...}` / `{"_warning": ...}` /
//!   `{"_error": ...}` lines with the exact field sets the old
//!   middleware/sites emitted (start/ok/error/error_detail;
//!   initialize_failed/auto_sync_*/...; mcp_serve_failed).
//!
//! Warning-shaped events flow through this layer exactly like `_log` events:
//! a site emits a `warn!`/`info!` event carrying `_warning` + structured
//! fields plus a site-controlled `warning_text` line (rendered verbatim in
//! text mode so prefixes like `timeline:` survive byte-for-byte; excluded
//! from JSON).
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
/// The internal `warning_text` field (text-mode-only hint, site-controlled)
/// is excluded — it is not part of the JSON contract.
struct JsonVisitor<'a>(&'a mut serde_json::Map<String, serde_json::Value>);

impl<'a> JsonVisitor<'a> {
    fn insert(&mut self, field: &Field, value: serde_json::Value) {
        if field.name() != "warning_text" {
            self.0.insert(field.name().to_string(), value);
        }
    }
}

impl<'a> Visit for JsonVisitor<'a> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.insert(field, json!(value));
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.insert(field, serde_json::Value::Number(value.into()));
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.insert(field, serde_json::Value::Number(value.into()));
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.insert(field, json!(value));
    }
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        // `%field` (Display) events arrive here via a Debug wrapper whose
        // Debug impl delegates to Display, so the string is unquoted.
        self.insert(field, json!(format!("{value:?}")));
    }
}

/// Render the text-mode line for `_log` (middleware progress) and `_warning`
/// (site diagnostics) events, matching the pre-tracing formats. Returns
/// `None` for everyday events without a recognizable kind — callers must not
/// emit a blank line for them.
///
/// Priority:
/// 1. `warning_text` — the site-controlled full line (`warning: ...` or
///    `timeline: ...`), kept byte-identical to the old `eprintln!`.
/// 2. `_log` middleware rendering (`[req] module action ok in 12ms`).
/// 3. `_warning` fallback (`warning: {status}: {message}`) for events that
///    carry the structured fields but no pre-formatted text.
fn render_text(event: &Event<'_>) -> Option<String> {
    let mut f = TextFields::default();
    event.record(&mut f);
    if let Some(text) = f.warning_text {
        return Some(text);
    }
    if let Some(kind) = f._log.as_deref() {
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
        return Some(line);
    }
    if let Some(status) = f._warning.as_deref() {
        let detail = f.message.as_deref().unwrap_or("");
        return Some(format!("warning: {status}: {detail}"));
    }
    if let Some(status) = f._error.as_deref() {
        let detail = f.message.as_deref().unwrap_or("");
        return Some(format!("error: {status}: {detail}"));
    }
    None
}

#[derive(Default)]
struct TextFields {
    _log: Option<String>,
    _warning: Option<String>,
    _error: Option<String>,
    request_id: Option<String>,
    module: Option<String>,
    action: Option<String>,
    elapsed_ms: Option<u64>,
    message: Option<String>,
    warning_text: Option<String>,
}

impl Visit for TextFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "_log" => self._log = Some(value.to_string()),
            "_warning" => self._warning = Some(value.to_string()),
            "_error" => self._error = Some(value.to_string()),
            "request_id" => self.request_id = Some(value.to_string()),
            "module" => self.module = Some(value.to_string()),
            "action" => self.action = Some(value.to_string()),
            "message" => self.message = Some(value.to_string()),
            "warning_text" => self.warning_text = Some(value.to_string()),
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

    #[test]
    fn json_mode_preserves_warning_shape() {
        // `_warning` events render `{"_warning": ..., module, message}` with
        // the internal `warning_text` field excluded.
        let out = run_with(true, LevelFilter::WARN, || {
            tracing::warn!(
                target: "everyday",
                _warning = "initialize_failed",
                module = "mail",
                message = "keyring unavailable",
                warning_text = "warning: mail initialize failed: keyring unavailable",
            );
        });
        let v: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(v["_warning"], "initialize_failed");
        assert_eq!(v["module"], "mail");
        assert_eq!(v["message"], "keyring unavailable");
        assert!(
            v.get("warning_text").is_none(),
            "warning_text is internal and must not leak into JSON"
        );
    }

    #[test]
    fn text_mode_renders_site_controlled_warning_text() {
        // Byte-identical to the old `eprintln!` line, including non-standard
        // prefixes like `timeline:`.
        let out = run_with(false, LevelFilter::WARN, || {
            tracing::warn!(
                target: "everyday",
                _warning = "timeline_insert_failed",
                source = "mail",
                message = "db write: boom",
                warning_text = "timeline: insert_events failed for mail: db write: boom",
            );
        });
        assert_eq!(
            out.trim(),
            "timeline: insert_events failed for mail: db write: boom"
        );
    }

    #[test]
    fn warning_fallback_renders_when_no_text_hint() {
        // No `warning_text`: fall back to `warning: {status}: {message}`.
        let out = run_with(false, LevelFilter::WARN, || {
            tracing::warn!(
                target: "everyday",
                _warning = "auto_sync_failed",
                message = "connection reset",
            );
        });
        assert_eq!(out.trim(), "warning: auto_sync_failed: connection reset");
    }

    #[test]
    fn warn_level_shows_warnings_and_silences_info_notices() {
        // auto_sync success is info (`-v` only); failure is warn (always).
        let out = run_with(false, LevelFilter::WARN, || {
            tracing::info!(
                target: "everyday",
                _warning = "auto_sync_pushed",
                message = "3 file(s) pushed",
                warning_text = "warning: auto_sync_pushed: 3 file(s) pushed",
            );
            tracing::warn!(
                target: "everyday",
                _warning = "auto_sync_failed",
                message = "connection reset",
                warning_text = "warning: auto_sync_failed: connection reset",
            );
        });
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines,
            vec!["warning: auto_sync_failed: connection reset"],
            "only the warn-level notice renders at default WARN"
        );
    }

    #[test]
    fn info_level_restores_info_notices() {
        let out = run_with(false, LevelFilter::INFO, || {
            tracing::info!(
                target: "everyday",
                _warning = "auto_sync_pushed",
                message = "3 file(s) pushed",
                warning_text = "warning: auto_sync_pushed: 3 file(s) pushed",
            );
        });
        assert_eq!(out.trim(), "warning: auto_sync_pushed: 3 file(s) pushed");
    }

    #[test]
    fn json_mode_keeps_auto_sync_pushed_shape_at_info() {
        // `-v --json`: the auto_sync success notice renders the original
        // `{"_warning": "auto_sync_pushed", "message": ...}` shape — the
        // info-level JSON contract, not just text.
        let out = run_with(true, LevelFilter::INFO, || {
            tracing::info!(
                target: "everyday",
                _warning = "auto_sync_pushed",
                message = "3 file(s) pushed",
                warning_text = "warning: auto_sync_pushed: 3 file(s) pushed",
            );
        });
        let v: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(v["_warning"], "auto_sync_pushed");
        assert_eq!(v["message"], "3 file(s) pushed");
        assert!(v.get("warning_text").is_none());
    }

    #[test]
    fn json_mode_preserves_error_shape() {
        // `_error` events (mcp serve startup failures) render
        // `{"_error": ..., "message": ...}` with `warning_text` excluded.
        let out = run_with(true, LevelFilter::WARN, || {
            tracing::error!(
                target: "everyday",
                _error = "mcp_serve_failed",
                message = "mcp serve: registry not initialized",
                warning_text = "error: mcp serve: registry not initialized",
            );
        });
        let v: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(v["_error"], "mcp_serve_failed");
        assert_eq!(v["message"], "mcp serve: registry not initialized");
        assert!(v.get("warning_text").is_none());
    }

    #[test]
    fn error_fallback_renders_when_no_text_hint() {
        // No `warning_text` on an `_error` event: fall back to
        // `error: {status}: {message}` instead of silently dropping it.
        let out = run_with(false, LevelFilter::WARN, || {
            tracing::error!(
                target: "everyday",
                _error = "mcp_serve_failed",
                message = "boom",
            );
        });
        assert_eq!(out.trim(), "error: mcp_serve_failed: boom");
    }
}
