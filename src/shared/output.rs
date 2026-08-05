//! Unified output layer.
//!
//! Every module returns an [`Output`], rendered centrally by the host program
//! according to [`RenderMode`]. `--json` switches to [`RenderMode::Json`],
//! the primary mode for AI Agent interaction.
//! See [F001](../../docs/adr/F001-cli-shape.md).

use serde_json::{Value, json};

use crate::error::{AgentError, Result};

/// A typed cell value for tabular output.
///
/// Text mode renders every cell via its [`TypedValue::as_display`] string;
/// JSON mode emits the native JSON type — numbers stay numbers, booleans
/// stay booleans, `Null` stays `null` — instead of stringifying everything.
/// See [F012](../../docs/adr/F012-architecture-deepening-phase.md) P6.
#[derive(Debug, Clone)]
pub enum TypedValue {
    /// String cell (JSON: string).
    Text(String),
    /// Numeric cell (JSON: number). Integral values render without a
    /// decimal point in text mode.
    Number(f64),
    /// Boolean cell (JSON: boolean).
    Boolean(bool),
    /// Empty/absent cell (JSON: null; text mode: empty string).
    Null,
}

impl TypedValue {
    /// Build a text cell.
    pub fn text<S: Into<String>>(s: S) -> Self {
        Self::Text(s.into())
    }

    /// Build a numeric cell.
    pub fn number(n: f64) -> Self {
        Self::Number(n)
    }

    /// Build a boolean cell.
    pub fn boolean(b: bool) -> Self {
        Self::Boolean(b)
    }

    /// The empty cell.
    pub fn null() -> Self {
        Self::Null
    }

    /// String form used by text-mode tables.
    pub fn as_display(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Number(n) => n.to_string(),
            Self::Boolean(b) => b.to_string(),
            Self::Null => String::new(),
        }
    }

    /// JSON value preserving the native type.
    ///
    /// Integral values are serialized as JSON integers (not `12345.0`):
    /// serde_json serializes `f64` with a trailing `.0`, which would leak into
    /// the output and disagree with the text-mode rendering. `uid`-style u32
    /// values (and anything within i64's exact range) stay clean integers.
    pub fn to_json(&self) -> Value {
        match self {
            Self::Text(s) => Value::String(s.clone()),
            Self::Number(n) => {
                if n.is_finite() && n.fract() == 0.0 && n.abs() < I64_EXACT_MAX {
                    json!(*n as i64)
                } else {
                    json!(n)
                }
            }
            Self::Boolean(b) => Value::Bool(*b),
            Self::Null => Value::Null,
        }
    }
}

/// Largest `f64` magnitude exactly representable as `i64` (2^53).
const I64_EXACT_MAX: f64 = 9_007_199_254_740_992.0;

/// Module execution result.
///
/// The four variants cover the typical output scenarios of a CLI tool:
/// - [`Output::Text`]: free-form text (e.g. cleaned-up Markdown)
/// - [`Output::Json`]: structured data (AI-friendly)
/// - [`Output::Records`]: tabular data — rendered as an aligned table in
///   Text mode, as an array of objects in JSON mode
/// - [`Output::TypedRecords`]: tabular data with typed cells — the same
///   rendering as `Records`, but JSON mode preserves numeric/boolean/null
///   types instead of converting everything to strings ([F012] P6)
#[derive(Debug, Clone)]
pub enum Output {
    /// Plain text. Emitted verbatim in both modes.
    Text(String),

    /// Structured JSON value.
    Json(Value),

    /// Tabular data: headers + rows.
    /// Text mode → aligned table; JSON mode → array of objects (headers as keys).
    Records {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },

    /// Tabular data with typed cells.
    /// Text mode → aligned table (cells via [`TypedValue::as_display`]);
    /// JSON mode → array of objects with native JSON value types.
    TypedRecords {
        headers: Vec<String>,
        rows: Vec<Vec<TypedValue>>,
    },
}

/// Render mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    /// Human-readable (default).
    Text,
    /// Clean JSON (`--json`).
    Json,
}

impl Output {
    /// Build a simple text output.
    pub fn text<S: Into<String>>(s: S) -> Self {
        Self::Text(s.into())
    }

    /// Build a tabular output.
    pub fn records(headers: Vec<String>, rows: Vec<Vec<String>>) -> Self {
        Self::Records { headers, rows }
    }

    /// Build a tabular output with typed cells (JSON mode preserves types).
    pub fn typed_records(headers: Vec<String>, rows: Vec<Vec<TypedValue>>) -> Self {
        Self::TypedRecords { headers, rows }
    }

    /// Render into a string under the given mode.
    ///
    /// JSON serialization failures no longer silently fall back to a
    /// `Value::Display` string (which would break the `--json` contract and
    /// make downstream Agents fail to parse). On failure we fall back to a
    /// JSON object tagged with an error marker instead.
    /// See [R002](../../docs/adr/R002-output-json-failure.md).
    pub fn render(self, mode: RenderMode) -> String {
        match (self, mode) {
            (Output::Text(s), _) => s,
            (Output::Json(v), RenderMode::Json) => compact_json(&v),
            (Output::Json(v), RenderMode::Text) => match serde_json::to_string_pretty(&v) {
                Ok(s) => s,
                Err(e) => fallback_json(&format!("serialize json value: {e}")),
            },
            (Output::Records { headers, rows }, RenderMode::Text) => render_table(&headers, &rows),
            (Output::Records { headers, rows }, RenderMode::Json) => {
                records_to_json(&headers, &rows)
            }
            (Output::TypedRecords { headers, rows }, RenderMode::Text) => {
                let str_rows: Vec<Vec<String>> = rows
                    .iter()
                    .map(|r| r.iter().map(TypedValue::as_display).collect())
                    .collect();
                render_table(&headers, &str_rows)
            }
            (Output::TypedRecords { headers, rows }, RenderMode::Json) => {
                typed_records_to_json(&headers, &rows)
            }
        }
    }
}

/// Render an error into an output string. JSON mode emits the format
/// mandated by [agents.md](../../agents.md).
pub fn render_error(err: &AgentError, mode: RenderMode) -> String {
    match mode {
        RenderMode::Json => match serde_json::to_string(err) {
            Ok(s) => s,
            Err(e) => fallback_json(&format!("serialize error envelope: {e}")),
        },
        RenderMode::Text => format!("error[{}]: {}", err.type_name(), err.message()),
    }
}

/// Fallback for serialization failure: return a JSON object tagged with
/// an `_error` field so downstream still recognizes it as JSON.
fn fallback_json(msg: &str) -> String {
    json!({ "_error": "serialize_failed", "message": msg }).to_string()
}

/// Resolve the render mode from the `--json` flag.
pub fn mode_from_json_flag(json: bool) -> RenderMode {
    if json {
        RenderMode::Json
    } else {
        RenderMode::Text
    }
}

/// Reduce a `Result<Output>` into `(exit_code, output_string)`.
pub fn finalize(result: Result<Output>, mode: RenderMode) -> (i32, String) {
    match result {
        Ok(out) => (0, out.render(mode)),
        Err(err) => (1, render_error(&err, mode)),
    }
}

fn compact_json(v: &Value) -> String {
    // Compact JSON, no extra whitespace — AI-parse-friendly.
    // On failure, no longer fall back to a non-JSON string (breaks the
    // contract); route to `fallback_json` instead.
    match serde_json::to_string(v) {
        Ok(s) => s,
        Err(e) => fallback_json(&format!("serialize json value: {e}")),
    }
}

fn records_to_json(headers: &[String], rows: &[Vec<String>]) -> String {
    rows_to_json(headers, rows, |v| Value::String(v.clone()))
}

/// Typed rows → array of JSON objects, preserving each cell's native type.
fn typed_records_to_json(headers: &[String], rows: &[Vec<TypedValue>]) -> String {
    rows_to_json(headers, rows, TypedValue::to_json)
}

/// Rows → array of JSON objects (headers as keys, cells mapped by `cell`).
fn rows_to_json<'a, T, F>(headers: &[String], rows: &'a [Vec<T>], cell: F) -> String
where
    F: Fn(&'a T) -> Value,
{
    let arr: Vec<Value> = rows
        .iter()
        .map(|row| {
            let mut obj = serde_json::Map::new();
            for (h, v) in headers.iter().zip(row.iter()) {
                obj.insert(h.clone(), cell(v));
            }
            Value::Object(obj)
        })
        .collect();
    compact_json(&Value::Array(arr))
}

/// Minimal table rendering: compute column widths + align. Works
/// without extra dependencies.
fn render_table(headers: &[String], rows: &[Vec<String>]) -> String {
    if headers.is_empty() {
        return String::new();
    }
    let ncol = headers.len();
    let mut widths = vec![0usize; ncol];
    for (i, h) in headers.iter().enumerate() {
        widths[i] = widths[i].max(display_width(h));
    }
    for row in rows {
        for (i, cell) in row.iter().enumerate().take(ncol) {
            widths[i] = widths[i].max(display_width(cell));
        }
    }

    let sep_len: usize = widths.iter().sum::<usize>() + 2 * ncol.saturating_sub(1);

    let render_line = |cells: &[String]| -> String {
        let mut line = String::new();
        for (i, (cell, w)) in cells.iter().zip(widths.iter()).enumerate() {
            if i > 0 {
                line.push_str("  ");
            }
            line.push_str(&pad(cell, *w));
        }
        line
    };

    let mut out = String::new();
    out.push_str(&render_line(headers));
    out.push('\n');
    out.push_str(&"-".repeat(sep_len));
    out.push('\n');
    for row in rows {
        out.push_str(&render_line(row));
        out.push('\n');
    }
    out
}

fn pad(s: &str, w: usize) -> String {
    let pad_len = w.saturating_sub(s.chars().count());
    format!("{}{}", s, " ".repeat(pad_len))
}

fn display_width(s: &str) -> usize {
    // Simplified: count by char. CJK width alignment is slightly off in
    // terminals; could be replaced with unicode-width later.
    s.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_output_unchanged_in_both_modes() {
        let out = Output::text("hello");
        assert_eq!(out.clone().render(RenderMode::Text), "hello");
        assert_eq!(out.render(RenderMode::Json), "hello");
    }

    #[test]
    fn json_output_compact_in_json_mode() {
        let out = Output::Json(json!({"a": 1, "b": [2, 3]}));
        assert_eq!(out.render(RenderMode::Json), r#"{"a":1,"b":[2,3]}"#);
    }

    #[test]
    fn records_to_json_array_of_objects() {
        let out = Output::records(
            vec!["name".into(), "age".into()],
            vec![vec!["alice".into(), "30".into()]],
        );
        let s = out.render(RenderMode::Json);
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v[0]["name"], "alice");
        assert_eq!(v[0]["age"], "30");
    }

    #[test]
    fn records_text_mode_has_header_and_separator() {
        let out = Output::records(vec!["k".into()], vec![vec!["v".into()]]);
        let s = out.render(RenderMode::Text);
        assert!(s.contains("k"));
        // Separator line: one whole line contains only '-'
        assert!(
            s.lines()
                .any(|l| !l.is_empty() && l.chars().all(|c| c == '-'))
        );
        assert!(s.contains("v"));
    }

    #[test]
    fn typed_records_json_preserves_native_types() {
        let out = Output::typed_records(
            vec!["uid".into(), "seen".into(), "folder".into(), "note".into()],
            vec![vec![
                TypedValue::Number(42.0),
                TypedValue::Boolean(true),
                TypedValue::text("INBOX"),
                TypedValue::Null,
            ]],
        );
        let s = out.render(RenderMode::Json);
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v[0]["uid"], json!(42)); // integral number serializes as integer
        assert_eq!(v[0]["seen"], json!(true));
        assert_eq!(v[0]["folder"], json!("INBOX"));
        assert!(v[0]["note"].is_null());
    }

    #[test]
    fn typed_records_text_renders_aligned_table() {
        let out = Output::typed_records(
            vec!["name".into(), "age".into()],
            vec![vec![TypedValue::text("alice"), TypedValue::Number(30.0)]],
        );
        let s = out.render(RenderMode::Text);
        assert!(s.contains("alice"));
        assert!(s.contains("30")); // integral number renders without ".0"
        assert!(
            s.lines()
                .any(|l| !l.is_empty() && l.chars().all(|c| c == '-'))
        );
    }

    #[test]
    fn typed_value_display_and_json_roundtrip() {
        assert_eq!(TypedValue::Number(3.5).as_display(), "3.5");
        assert_eq!(TypedValue::Number(3.0).as_display(), "3");
        assert_eq!(TypedValue::Boolean(false).as_display(), "false");
        assert_eq!(TypedValue::Null.as_display(), "");
        assert_eq!(TypedValue::Null.to_json(), Value::Null);
        assert_eq!(TypedValue::Number(1.5).to_json(), json!(1.5));
        assert_eq!(TypedValue::Number(42.0).to_json(), json!(42)); // no ".0" leak
        assert_eq!(TypedValue::Number(-7.0).to_json(), json!(-7));
        assert_eq!(TypedValue::Boolean(true).to_json(), json!(true));
        // f64 beyond i64's exact range keeps floating-point serialization.
        assert_eq!(TypedValue::Number(1e300).to_json(), json!(1e300));
    }

    #[test]
    fn error_renders_spec_json_format() {
        let err = AgentError::Network("timeout".into());
        let s = render_error(&err, RenderMode::Json);
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["error"], "NetworkError");
        assert_eq!(v["message"], "network error: timeout");
    }

    #[test]
    fn finalize_ok_returns_zero() {
        let (code, _) = finalize(Ok(Output::text("ok")), RenderMode::Text);
        assert_eq!(code, 0);
    }

    #[test]
    fn finalize_err_returns_nonzero() {
        let (code, _) = finalize(Err(AgentError::Other("x".into())), RenderMode::Text);
        assert_ne!(code, 0);
    }
}
