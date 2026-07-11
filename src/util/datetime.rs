//! 时间解析工具。
//!
//! 三个模块（email_cache / timeline/store / timeline/providers）原本各自
//! 定义了 `parse_rfc3339`，实现完全相同。集中到此处统一行为。

use chrono::{DateTime, Utc};

/// 解析 RFC3339 字符串为 UTC `DateTime`。
///
/// 失败返回 `None` —— 调用方必须显式处理（不要静默 fallback 到 `Utc::now()`，
/// 否则无法区分"刚 sync 过"和"水位损坏"）。
pub fn parse_rfc3339(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_z_suffix_as_utc() {
        let dt = parse_rfc3339("2026-07-11T14:30:00Z").expect("Z must parse");
        // 用 parse_from_rfc3339 自身作 oracle，避免硬编码 Unix timestamp
        let expected = DateTime::parse_from_rfc3339("2026-07-11T14:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(dt, expected);
    }

    #[test]
    fn parses_explicit_offset() {
        let dt = parse_rfc3339("2026-07-11T14:30:00+08:00").expect("offset must parse");
        // +08:00 = 06:30 UTC，等价于 14:30:00Z 减去 8 小时。
        let expected = DateTime::parse_from_rfc3339("2026-07-11T06:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(dt, expected);
    }

    #[test]
    fn invalid_string_returns_none() {
        assert!(parse_rfc3339("not a date").is_none());
        assert!(parse_rfc3339("").is_none());
        assert!(parse_rfc3339("2026-13-99T99:99:99Z").is_none());
    }
}