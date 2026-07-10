//! 命令行参数解析工具。

use std::collections::HashMap;

/// 解析 `--flag value` 形式的简单参数。
/// 返回 (flags map, positional args)。
/// 模块可复用此工具函数，避免每个模块都引入 clap。
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
                // --key value
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
        // 约定：`--flag` 后跟非 `--` token 会被当作该 flag 的值；
        // 布尔 flag 必须后接另一个 `--flag` 或位于末尾。
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
}
