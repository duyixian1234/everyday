//! CLI argument-parsing utilities.

use std::collections::HashMap;

/// Parse simple `--flag value` style arguments.
/// Returns `(flags map, positional args)`.
/// Modules reuse this helper to avoid pulling in `clap` per module.
///
/// Key contract: a token following `--flag` is treated as that flag's value
/// only when it does not itself start with `--`. See
/// [R005](../../docs/adr/R005-parse-simple-args.md).
///
/// - `--limit 10`      → limit=10, because "10" does not start with `--`
/// - `--limit --other` → limit=true (boolean), because "--other" is a flag
/// - `--limit -1`      → limit="-1", negative numbers are also values
/// - `--account -X`    → account="-X", single-dash tokens are values
///
/// The old implementation misread `-1`/`-X` as the start of the next flag,
/// turning the preceding flag into a boolean.
pub fn parse_simple_args(args: &[String]) -> (HashMap<String, String>, Vec<String>) {
    let mut flags = HashMap::new();
    let mut positional = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(stripped) = a.strip_prefix("--") {
            if let Some((k, v)) = stripped.split_once('=') {
                flags.insert(k.to_string(), v.to_string());
            } else if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                flags.insert(stripped.to_string(), args[i + 1].clone());
                i += 1;
            } else {
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
        // Before the fix, `--limit -1` treated -1 as the start of another flag,
        // swallowing limit into "true". After the fix, the single-dash token
        // (-1) is passed to limit as a value.
        let args: Vec<String> = ["--limit", "-1"].iter().map(|s| s.to_string()).collect();
        let (flags, _) = parse_simple_args(&args);
        assert_eq!(flags.get("limit"), Some(&"-1".to_string()));
    }

    #[test]
    fn single_dash_token_is_kept_as_value() {
        // `--account -X` treats -X as the account name (value).
        let args: Vec<String> = ["--account", "-X"].iter().map(|s| s.to_string()).collect();
        let (flags, _) = parse_simple_args(&args);
        assert_eq!(flags.get("account"), Some(&"-X".to_string()));
    }

    #[test]
    fn long_flag_before_another_long_flag_is_boolean() {
        // `--unread --limit 5`: unread is followed by --limit (starts with --),
        // so unread is boolean; limit=5.
        let args: Vec<String> = ["--unread", "--limit", "5"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (flags, _) = parse_simple_args(&args);
        assert_eq!(flags.get("unread"), Some(&"true".to_string()));
        assert_eq!(flags.get("limit"), Some(&"5".to_string()));
    }
}
