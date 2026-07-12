//! String helpers.

/// Truncate `s` to at most `n` Unicode scalar values (chars), always slicing
/// on a UTF-8 char boundary.
///
/// Unlike `&s[..n]` (which counts *bytes* and panics when `n` lands inside a
/// multi-byte character such as a CJK ideograph), this keeps whole chars, so
/// e.g. Chinese RSS summaries can never trigger a "not a char boundary" panic.
pub fn truncate_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_respects_char_boundary() {
        // 300 Chinese chars => byte length 900. The old `&s[..200]` would
        // slice mid-character and panic; truncate_chars must keep whole chars.
        let s = "测".repeat(300);
        assert!(s.len() > 200);
        let t = truncate_chars(&s, 200);
        assert_eq!(t.chars().count(), 200);
        // Re-slicing the result must never panic.
        assert_eq!(&t[..t.len()], t);
    }

    #[test]
    fn truncate_keeps_short_string() {
        let s = "hello 世界";
        assert_eq!(truncate_chars(s, 500), s);
        assert_eq!(truncate_chars("", 10), "");
    }

    #[test]
    fn truncate_exact_boundary() {
        let s = "abcdef";
        assert_eq!(truncate_chars(s, 3), "abc");
        assert_eq!(truncate_chars(s, 6), s);
    }
}
