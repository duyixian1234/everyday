//! Short unique ID generation utilities.

use std::sync::atomic::{AtomicU64, Ordering};

/// Process-local monotonic counter; guarantees uniqueness even when two
/// calls share the same nanosecond timestamp.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a prefixed short unique ID (nanosecond timestamp + process-local
/// counter; stays unique even inside a tight loop).
///
/// Example: `gen_id("n")` → `n17abc...-1`, `gen_id("t")` → `t17abc...-2`.
pub fn gen_id(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}{nanos:x}-{seq:x}")
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
}
