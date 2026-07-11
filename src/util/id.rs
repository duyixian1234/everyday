//! 短唯一 ID 生成工具。

use std::sync::atomic::{AtomicU64, Ordering};

/// 进程内单调递增计数器，保证纳秒相同时仍唯一。
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 生成带前缀的短唯一 ID（纳秒时间戳 + 进程内计数器，循环内亦唯一）。
///
/// 例：`gen_id("n")` → `n17abc...-1`、`gen_id("t")` → `t17abc...-2`。
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
        // 关键回归：纳秒相同时（旧实现会撞）必须仍唯一。
        let mut seen = HashSet::new();
        for _ in 0..1000 {
            let id = gen_id("x");
            assert!(seen.insert(id.clone()), "duplicate id: {id}");
        }
    }
}
