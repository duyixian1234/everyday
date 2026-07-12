//! Datetime parsing utilities.
//!
//! Three modules (`email_cache` / `timeline/store` / `timeline/providers`)
//! each previously defined their own `parse_rfc3339` with identical bodies.
//! Centralized here to unify behavior.
//!
//! [`parse_since`] is the shared `--since` flag parser used by both the
//! `timeline` and the new `search` modules
//! ([L012](../../docs/adr/L012-since-query-flag.md),
//! [S006](../../docs/adr/S006-search-module-cli.md)).

use chrono::{DateTime, Local, TimeZone, Utc};

use crate::error::{AgentError, Result};

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

/// Parse `--since YYYY-MM-DD` or relative duration `30m`/`2h`/`1d`/`7d`
/// into a UTC `DateTime`. Sub-day precision is preserved: `--since 30m`
/// returns `now - 30 minutes`.
///
/// Date form: 00:00 local of that date → UTC.
/// Relative form: `now - duration` (1-minute granularity for `m`, exact
/// for `h`/`d`).
///
/// This was previously a private helper in `timeline.rs`
/// ([L012](../../docs/adr/L012-since-query-flag.md)); moved to `util`
/// so the `search` module can reuse it
/// ([S006](../../docs/adr/S006-search-module-cli.md)).
pub fn parse_since(s: &str) -> Result<DateTime<Utc>> {
    let s = s.trim();
    // 1. Date
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let ndt = d.and_hms_opt(0, 0, 0).ok_or_else(|| {
            AgentError::InvalidArgument(format!("invalid --since '{s}': date-time build failed"))
        })?;
        return Local
            .from_local_datetime(&ndt)
            .earliest()
            .map(|dt| dt.with_timezone(&Utc))
            .ok_or_else(|| {
                AgentError::InvalidArgument(format!("invalid --since '{s}': DST gap on date"))
            });
    }
    // 2. Relative duration
    if s.len() >= 2 {
        let (num, unit) = s.split_at(s.len() - 1);
        if let Ok(n) = num.parse::<u64>() {
            let now_local = Local::now();
            let dt = match unit {
                "m" => now_local - chrono::Duration::minutes(n as i64),
                "h" => now_local - chrono::Duration::hours(n as i64),
                "d" => now_local - chrono::Duration::days(n as i64),
                _ => {
                    return Err(AgentError::InvalidArgument(format!(
                        "invalid --since '{s}', expected YYYY-MM-DD or 30m/2h/1d"
                    )));
                }
            };
            return Ok(dt.with_timezone(&Utc));
        }
    }
    Err(AgentError::InvalidArgument(format!(
        "invalid --since '{s}', expected YYYY-MM-DD or 30m/2h/1d"
    )))
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

    #[test]
    fn parse_since_date_form_returns_utc() {
        let dt = parse_since("2026-07-11").unwrap();
        let now = Utc::now();
        assert!(dt < now);
        assert!(dt < now + chrono::Duration::days(1));
    }

    #[test]
    fn parse_since_duration_days_subtracts() {
        let now = Utc::now();
        let dt = parse_since("1d").unwrap();
        assert!(dt < now);
        assert!(dt > now - chrono::Duration::days(2));
    }

    #[test]
    fn parse_since_duration_minutes_subtracts() {
        let now = Utc::now();
        let dt = parse_since("30m").unwrap();
        let diff = now - dt;
        assert!(diff >= chrono::Duration::minutes(29));
        assert!(diff <= chrono::Duration::minutes(31));
    }

    #[test]
    fn parse_since_invalid_errors() {
        assert!(parse_since("30x").is_err());
        assert!(parse_since("not-a-thing").is_err());
        assert!(parse_since("2026/07/11").is_err());
    }
}
