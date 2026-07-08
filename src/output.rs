//! 统一输出层。
//!
//! 所有模块返回 [`Output`]，由主程序根据 [`RenderMode`] 统一渲染。
//! `--json` 切换到 [`RenderMode::Json`]，这是 AI Agent 交互的主模式。

use serde_json::{json, Value};

use crate::error::{AgentError, Result};

/// 模块执行结果。
///
/// 三种变体覆盖 CLI 工具的典型输出场景：
/// - [`Output::Text`]：自由文本（如 `net fetch` 清洗后的 Markdown）
/// - [`Output::Json`]：结构化数据（AI 友好）
/// - [`Output::Records`]：表格型数据，Text 模式渲染为表格，JSON 模式渲染为对象数组
#[derive(Debug, Clone)]
pub enum Output {
    /// 纯文本。两种模式都原样输出。
    Text(String),

    /// 结构化 JSON 值。
    Json(Value),

    /// 表格型数据：表头 + 行。
    /// Text 模式 → 对齐表格；JSON 模式 → 对象数组（表头作键）。
    Records {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
}

/// 渲染模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    /// 人类可读（默认）。
    Text,
    /// 纯净 JSON（`--json`）。
    Json,
}

impl Output {
    /// 创建一个简单的文本输出。
    pub fn text<S: Into<String>>(s: S) -> Self {
        Self::Text(s.into())
    }

    /// 创建一个 JSON 输出。
    pub fn json<V: Into<Value>>(v: V) -> Self {
        Self::Json(v.into())
    }

    /// 创建一个表格输出。
    pub fn records(headers: Vec<String>, rows: Vec<Vec<String>>) -> Self {
        Self::Records { headers, rows }
    }

    /// 按指定模式渲染为字符串。
    pub fn render(self, mode: RenderMode) -> String {
        match (self, mode) {
            (Output::Text(s), _) => s,
            (Output::Json(v), RenderMode::Json) => compact_json(&v),
            (Output::Json(v), RenderMode::Text) => serde_json::to_string_pretty(&v)
                .unwrap_or_else(|_| v.to_string()),
            (Output::Records { headers, rows }, RenderMode::Text) => render_table(&headers, &rows),
            (Output::Records { headers, rows }, RenderMode::Json) => records_to_json(&headers, &rows),
        }
    }
}

/// 将错误渲染为输出字符串。JSON 模式输出 PRD 规定格式。
pub fn render_error(err: &AgentError, mode: RenderMode) -> String {
    match mode {
        RenderMode::Json => serde_json::to_string(err).unwrap_or_else(|_| {
            json!({
                "error": err.type_name(),
                "message": err.message()
            })
            .to_string()
        }),
        RenderMode::Text => format!("error[{}]: {}", err.type_name(), err.message()),
    }
}

/// 解析渲染模式（从 `--json` flag）。
pub fn mode_from_json_flag(json: bool) -> RenderMode {
    if json {
        RenderMode::Json
    } else {
        RenderMode::Text
    }
}

/// 把 `Result<Output>` 统一转成 `(退出码, 输出字符串)`。
pub fn finalize(result: Result<Output>, mode: RenderMode) -> (i32, String) {
    match result {
        Ok(out) => (0, out.render(mode)),
        Err(err) => (1, render_error(&err, mode)),
    }
}

fn compact_json(v: &Value) -> String {
    // 紧凑 JSON，无多余空白 —— AI 解析友好。
    serde_json::to_string(v).unwrap_or_else(|_| v.to_string())
}

fn records_to_json(headers: &[String], rows: &[Vec<String>]) -> String {
    let arr: Vec<Value> = rows
        .iter()
        .map(|row| {
            let mut obj = serde_json::Map::new();
            for (h, v) in headers.iter().zip(row.iter()) {
                obj.insert(h.clone(), Value::String(v.clone()));
            }
            Value::Object(obj)
        })
        .collect();
    compact_json(&Value::Array(arr))
}

/// 极简表格渲染：计算列宽 + 对齐。不引入额外依赖即可工作。
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
    // 简化：按字符计数。CJK 宽度对齐在终端略有偏差，可后续替换为 unicode-width。
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
        let out = Output::json(json!({"a": 1, "b": [2, 3]}));
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
        let out = Output::records(
            vec!["k".into()],
            vec![vec!["v".into()]],
        );
        let s = out.render(RenderMode::Text);
        assert!(s.contains("k"));
        // 分隔线：一整行只含 '-'
        assert!(s.lines().any(|l| !l.is_empty() && l.chars().all(|c| c == '-')));
        assert!(s.contains("v"));
    }

    #[test]
    fn error_renders_prd_json_format() {
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
