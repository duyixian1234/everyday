//! 短唯一 ID 生成工具。

/// 生成带前缀的短唯一 ID（基于纳秒时间戳的十六进制；CLI 串行调用下足够唯一）。
///
/// 例：`gen_id("n")` → `n17abc...`、`gen_id("t")` → `t17abc...`。
pub fn gen_id(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{prefix}{nanos:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gen_id_uses_prefix() {
        assert!(gen_id("n").starts_with('n'));
        assert!(gen_id("t").starts_with('t'));
    }
}
