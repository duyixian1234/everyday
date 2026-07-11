//! 命令行参数解析工具。

use std::collections::HashMap;

/// 解析 `--flag value` 形式的简单参数。
/// 返回 (flags map, positional args)。
/// 模块可复用此工具函数，避免每个模块都引入 clap。
///
/// 关键约定：跟在 `--flag` 之后的 token 只有在不以 `--` 开头时才被
/// 当作该 flag 的值。
///
/// - `--limit 10`         → limit=10, 因为 "10" 不以 `--` 开头
/// - `--limit --other`    → limit=true (boolean), 因为 "--other" 是 flag
/// - `--limit -1`         → limit="-1", 负数也作为值
/// - `--account -X`       → account="-X", 单破折号 token 作为值
///
/// 老实现误把 `-1`/`-X` 视为下一个 flag，把前一个 flag 设为 boolean。
pub fn parse_simple_args(args: &[String]) -> (HashMap<String, String>, Vec<String>) {
    let mut flags = HashMap::new();
    let mut positional = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(stripped) = a.strip_prefix("--") {
            // --key=value
            if let Some((k, v)) = stripped.split_once('=') {
                flags.insert(k.to_string(), v.to_string());
            } else if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                // --key value：值不以 `--` 开头（单破折号负数/短 flag 形式视为值）
                flags.insert(stripped.to_string(), args[i + 1].clone());
                i += 1;
            } else {
                // --flag (boolean)
                flags.insert(stripped.to_string(), "true".to_string());
            }
        } else {
            positional.push(a.clone());
        }
        i += 1;
    }
    (flags, positional)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_flags_and_positional() {
        let args: Vec<String> = ["--unread", "--limit", "10", "list", "extra"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (flags, positional) = parse_simple_args(&args);
        assert_eq!(flags.get("unread"), Some(&"true".to_string()));
        assert_eq!(flags.get("limit"), Some(&"10".to_string()));
        assert_eq!(positional, vec!["list", "extra"]);
    }

    #[test]
    fn parse_key_eq_value() {
        let args: Vec<String> = ["--limit=5", "pos"].iter().map(|s| s.to_string()).collect();
        let (flags, positional) = parse_simple_args(&args);
        assert_eq!(flags.get("limit"), Some(&"5".to_string()));
        assert_eq!(positional, vec!["pos"]);
    }

    #[test]
    fn negative_value_is_kept_as_value() {
        // 修复前：`--limit -1` 会把 -1 当成另一个 flag 的开头，limit 被吞为 "true"。
        // 修复后：单破折号 token（-1）作为值传给 limit。
        let args: Vec<String> = ["--limit", "-1"].iter().map(|s| s.to_string()).collect();
        let (flags, _) = parse_simple_args(&args);
        assert_eq!(flags.get("limit"), Some(&"-1".to_string()));
    }

    #[test]
    fn single_dash_token_is_kept_as_value() {
        // `--account -X` 把 -X 当成账号名（值）。
        let args: Vec<String> = ["--account", "-X"].iter().map(|s| s.to_string()).collect();
        let (flags, _) = parse_simple_args(&args);
        assert_eq!(flags.get("account"), Some(&"-X".to_string()));
    }

    #[test]
    fn long_flag_before_another_long_flag_is_boolean() {
        // `--unread --limit 5`：unread 后面是 --limit（以 -- 开头），
        // 所以 unread 是 boolean；limit=5。
        let args: Vec<String> = ["--unread", "--limit", "5"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (flags, _) = parse_simple_args(&args);
        assert_eq!(flags.get("unread"), Some(&"true".to_string()));
        assert_eq!(flags.get("limit"), Some(&"5".to_string()));
    }
}
