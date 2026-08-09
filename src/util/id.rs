//! Short unique ID generation utilities.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// Process-local monotonic counter; guarantees uniqueness even when two
/// calls share the same nanosecond timestamp.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// This process's PID, cached — the cross-process uniqueness component.
///
/// `cargo nextest` runs each test in its own process; without the PID segment
/// two processes starting in the same nanosecond would both begin their
/// counters at 0 and generate the **same** id (e.g. two tests sharing one
/// temp DB file → SQLite WAL readonly / row-count corruption). The PID makes
/// ids unique across processes as well.
static PID: LazyLock<u64> = LazyLock::new(|| std::process::id() as u64);

/// Generate a prefixed short unique ID (nanosecond timestamp + PID +
/// process-local counter; unique both within a process and across processes).
///
/// Example: `gen_id("n")` → `n17abc...-1a2b3-1`, `gen_id("t")` → `t17abc...-1a2b3-2`.
pub fn gen_id(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}{nanos:x}-{:x}-{seq:x}", *PID)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn gen_id_uses_prefix() {
        assert!(gen_id("n").starts_with('n'));
        assert!(gen_id("t").starts_with('t'));
    }

    #[test]
    fn gen_id_unique_within_loop() {
        // Key regression: when nanosecond timestamps coincide (the old
        // implementation collided), IDs must still be unique.
        let mut seen = HashSet::new();
        for _ in 0..1000 {
            let id = gen_id("x");
            assert!(seen.insert(id.clone()), "duplicate id: {id}");
        }
    }

    #[test]
    fn gen_id_embeds_pid() {
        let id = gen_id("x");
        // The PID segment is what keeps ids unique across nextest processes
        // (each test = one process, counters all start at 0). The seq suffix
        // is process-local, so it must not be pinned here.
        assert!(id.contains(&format!("-{:x}-", *PID)), "{id}");
    }
}
