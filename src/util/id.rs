//! Short unique ID generation utilities.
//!
//! Format: `{prefix}{YYYYMMDD}-{pid:x}-{seq}` — a local-calendar date, the
//! process PID (hex), and a per-prefix ordinal (zero-padded to 3 digits,
//! extending past 999). Decision record: docs/adr/R021-date-sequence-id.md.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::Mutex;

/// Per-process ID state: the last-observed local calendar day (`YYYYMMDD` as
/// u64) and the per-prefix ordinal counters for that day.
///
/// Counters reset when the observed day changes, keeping ids short and
/// self-explanatory (`n20260814-1a2b-001` = "the 1st note by process 1a2b on
/// 2026-08-14") while remaining unique. The PID segment — restored in the
/// R021 revision below — keeps ids unique **across processes**: the CLI is
/// one-shot per command, so two `everyday todo add` runs on the same day
/// would otherwise both start their ordinal at 001 and collide on
/// `t20260814-001` (surfaced by the SQLite `PRIMARY KEY` constraint).
struct IdState {
    day: u64,
    counters: HashMap<String, u64>,
}

static STATE: LazyLock<Mutex<IdState>> = LazyLock::new(|| {
    Mutex::new(IdState {
        day: 0,
        counters: HashMap::new(),
    })
});

/// Local calendar date as `YYYYMMDD` (u64), e.g. `20260814`.
fn local_day() -> u64 {
    chrono::Local::now()
        .format("%Y%m%d")
        .to_string()
        .parse()
        .unwrap_or(0)
}

/// Generate a prefixed date-sequence unique ID.
///
/// Example: `gen_id("n")` → `n20260814-1a2b-001`, `gen_id("t")` →
/// `t20260814-1a2b-001`. The ordinal restarts at 001 each local calendar day,
/// independently per prefix; it extends to 4+ digits if a prefix ever exceeds
/// 999 ids in a day.
pub fn gen_id(prefix: &str) -> String {
    gen_id_for_day(prefix, local_day())
}

/// Core generator with the day injected (tests pin a fixed day).
fn gen_id_for_day(prefix: &str, day: u64) -> String {
    let mut state = STATE.lock().unwrap();
    if state.day != day {
        state.day = day;
        state.counters.clear();
    }
    let seq = state.counters.entry(prefix.to_string()).or_insert(0);
    *seq += 1;
    format!("{prefix}{day}-{:x}-{seq:03}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Serialize tests touching the shared `STATE` — nextest runs tests in
    /// parallel within one process, and per-prefix ordinal assertions must not
    /// observe another test's increments.
    static TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    #[test]
    fn gen_id_uses_prefix() {
        let _guard = TEST_LOCK.lock().unwrap();
        assert!(gen_id("n").starts_with('n'));
        assert!(gen_id("t").starts_with('t'));
    }

    #[test]
    fn gen_id_unique_within_loop() {
        let _guard = TEST_LOCK.lock().unwrap();
        // Key regression: ids must stay unique within a process across 1000
        // rapid calls; the date segment also keeps them unique across the
        // midnight boundary.
        let mut seen = HashSet::new();
        for _ in 0..1000 {
            let id = gen_id("x");
            assert!(seen.insert(id.clone()), "duplicate id: {id}");
        }
    }

    #[test]
    fn gen_id_matches_date_seq_format() {
        let _guard = TEST_LOCK.lock().unwrap();
        // {prefix}{YYYYMMDD}-{pid:x}-{seq}: "ev" + 8-digit date + "-" + pid
        // (hex) + "-" + >=3-digit seq
        let id = gen_id("ev");
        let body = id.strip_prefix("ev").expect("prefix");
        let mut parts = body.split('-');
        let date = parts.next().expect("date segment");
        let pid = parts.next().expect("pid segment");
        let seq = parts.next().expect("seq segment");
        assert_eq!(date.len(), 8, "date must be YYYYMMDD: {id}");
        assert!(date.chars().all(|c| c.is_ascii_digit()), "{id}");
        assert!(
            pid.chars().all(|c| c.is_ascii_hexdigit()),
            "pid must be hex: {id}"
        );
        assert!(!pid.is_empty(), "{id}");
        assert!(seq.len() >= 3, "seq must be zero-padded >=3: {id}");
        assert!(seq.chars().all(|c| c.is_ascii_digit()), "{id}");
        assert!(parts.next().is_none(), "no extra segments: {id}");
    }

    #[test]
    fn gen_id_per_prefix_independent_on_same_day() {
        let _guard = TEST_LOCK.lock().unwrap();
        let pid = std::process::id();
        assert_eq!(
            gen_id_for_day("zz", 20260814),
            format!("zz20260814-{pid:x}-001")
        );
        assert_eq!(
            gen_id_for_day("zz", 20260814),
            format!("zz20260814-{pid:x}-002")
        );
        assert_eq!(
            gen_id_for_day("yy", 20260814),
            format!("yy20260814-{pid:x}-001")
        );
    }

    #[test]
    fn gen_id_resets_on_new_day() {
        let _guard = TEST_LOCK.lock().unwrap();
        let pid = std::process::id();
        assert_eq!(
            gen_id_for_day("zx", 20260814),
            format!("zx20260814-{pid:x}-001")
        );
        assert_eq!(
            gen_id_for_day("zx", 20260814),
            format!("zx20260814-{pid:x}-002")
        );
        assert_eq!(
            gen_id_for_day("zx", 20260815),
            format!("zx20260815-{pid:x}-001")
        );
    }
}
