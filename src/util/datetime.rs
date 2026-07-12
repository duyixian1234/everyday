//! Datetime parsing utilities.
//!
//! Three modules (`email_cache` / `timeline/store` / `timeline/providers`)
//! each previously defined their own `parse_rfc3339` with identical bodies.
//! Centralized here to unify behavior.

use chrono::{DateTime, Utc};

/// Parse an RFC3339 string into a UTC `DateTime`.
///
/// On failure returns `None` — the caller must handle it explicitly. Do **not**
/// silently fall back to `Utc::now()`, or it becomes impossible to tell
/// "just synced" apart from "watermark corrupted". See
/// [L013](../../docs/adr/L013-from-explicit-error.md).
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
        // Use parse_from_rfc3339 itself as the oracle to avoid hardcoding a
        // Unix timestamp.
        let expected = DateTime::parse_from_rfc3339("2026-07-11T14:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(dt, expected);
    }

    #[test]
    fn parses_explicit_offset() {
        let dt = parse_rfc3339("2026-07-11T14:30:00+08:00").expect("offset must parse");
        // +08:00 = 06:30 UTC, equivalent to 14:30:00Z minus 8 hours.
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
