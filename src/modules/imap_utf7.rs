//! IMAP modified UTF-7 folder names (RFC 3501 §5.1.3) — decode, encode, and
//! the canonical folder *key* used by the envelope cache.
//!
//! Why this is a standalone module: the cache's primary key is
//! `(account, folder, uid)`, so the *spelling* of `folder` is a storage
//! contract, not a display detail. Decoding alone lives in the mail module,
//! but the cache layer ([`crate::modules::email_cache`]) must be able to
//! canonicalize a caller-supplied folder name without depending on the whole
//! mail module. See [M008](../../docs/adr/M008-mail-folder-key-canonicalization.md).
//!
//! - `decode_imap_utf7(raw) -> display` — what users see (`其他文件夹/存档`).
//! - `encode_imap_utf7(display) -> raw` — the server's wire form
//!   (`&UXZO1mWHTvZZOQ-/&W1hoYw-`).
//! - [`canonical_folder_key`] — map either spelling onto the single cache key.

/// Decode an IMAP UTF-7 folder name (RFC 3501 §5.1.3) into readable UTF-8.
///
/// Rule: a segment starting with `&` and ending with `-` is modified base64
/// encoding of UTF-16BE; `&-` means a literal `&`; all other characters pass
/// through. We iterate by `char` to handle UTF-8 correctly (the user may pass a
/// Chinese name directly, with no `&` segment).
/// Example: `&UXZO1mWHTvZZOQ-/Github&kBp35Q-` → `其他文件夹/Github通知`.
pub fn decode_imap_utf7(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            let mut segment = String::new();
            let mut found_terminator = false;
            while let Some(&nc) = chars.peek() {
                chars.next();
                if nc == '-' {
                    found_terminator = true;
                    break;
                }
                segment.push(nc);
            }
            if !found_terminator {
                // no terminating '-', emit as-is
                out.push('&');
                out.push_str(&segment);
                break;
            }
            if segment.is_empty() {
                out.push('&'); // &- → literal &
            } else if let Some(decoded) = decode_modified_base64_utf16(segment.as_bytes()) {
                out.push_str(&decoded);
            } else {
                // decode failed, keep the original segment
                out.push('&');
                out.push_str(&segment);
                out.push('-');
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Encode a readable UTF-8 folder name into IMAP modified UTF-7.
///
/// Rule (inverse of [`decode_imap_utf7`]): printable ASCII `0x20..=0x7E` except
/// `&` is emitted directly; `&` becomes `&-`; everything else (non-ASCII and
/// control characters) is collected into a shift run encoded as modified base64
/// of UTF-16BE, wrapped in `&` … `-` with no padding.
/// Example: `其他文件夹/存档` → `&UXZO1mWHTvZZOQ-/&W1hoYw-`.
pub fn encode_imap_utf7(s: &str) -> String {
    let mut out = String::new();
    let mut shift: Vec<u16> = Vec::new();
    for c in s.chars() {
        if c == '&' {
            flush_shift_run(&mut out, &mut shift);
            out.push_str("&-");
        } else if ('\u{20}'..='\u{7e}').contains(&c) {
            flush_shift_run(&mut out, &mut shift);
            out.push(c);
        } else {
            let mut buf = [0u16; 2];
            for unit in c.encode_utf16(&mut buf) {
                shift.push(*unit);
            }
        }
    }
    flush_shift_run(&mut out, &mut shift);
    out
}

/// Canonical cache key for a folder name — the single spelling under which an
/// envelope or watermark is stored.
///
/// A folder reaches `everyday` in either spelling: IMAP `LIST` returns the raw
/// modified-UTF-7 name, while `mail folders` and users speak decoded display
/// names. Storing both spellings as separate keys splits one physical folder
/// into two cache namespaces: the same message lands on two rows (visible as a
/// phantom "duplicate delivery"), and the folder carries two watermark rows.
///
/// Rule:
/// - already canonical (raw name, or plain ASCII) → returned unchanged;
/// - otherwise treated as a display name → the raw form is returned.
///
/// `INBOX` is folded to upper case: RFC 3501 §5.1 defines INBOX as
/// case-insensitive, so `--folder inbox` must not create a second key.
pub fn canonical_folder_key(name: &str) -> String {
    if name.eq_ignore_ascii_case("INBOX") {
        return "INBOX".to_string();
    }
    let candidate = encode_imap_utf7(&decode_imap_utf7(name));
    // Round-trips → the input was already the canonical raw name (or plain
    // ASCII); keep it byte-for-byte rather than rewriting a valid key.
    if candidate == name {
        name.to_string()
    } else {
        candidate
    }
}

/// Emit the pending UTF-16 units as one `&…-` shift run, then clear it.
fn flush_shift_run(out: &mut String, shift: &mut Vec<u16>) {
    if shift.is_empty() {
        return;
    }
    let mut bytes = Vec::with_capacity(shift.len() * 2);
    for unit in shift.iter() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    out.push('&');
    out.push_str(&encode_base64_modified(&bytes));
    out.push('-');
    shift.clear();
}

/// modified base64 (`,` replaces `/`, no padding) → UTF-16BE → String.
fn decode_modified_base64_utf16(b64: &[u8]) -> Option<String> {
    let raw = decode_base64_modified(b64)?;
    if raw.len() % 2 != 0 {
        return None;
    }
    let u16s: Vec<u16> = raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_be_bytes(*c))
        .collect();
    String::from_utf16(&u16s).ok()
}

/// modified base64 decode (dependency-free, hand-written).
fn decode_base64_modified(input: &[u8]) -> Option<Vec<u8>> {
    const TABLE: [i8; 256] = build_b64_table();
    let mut out = Vec::new();
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for &c in input {
        if c == b'=' {
            break;
        }
        let v = TABLE[c as usize];
        if v < 0 {
            continue;
        }
        buf = (buf << 6) | (v as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

/// modified base64 encode (`,` replaces `/`, no padding) — inverse of
/// [`decode_base64_modified`].
fn encode_base64_modified(bytes: &[u8]) -> String {
    // index 62 = '+', index 63 = ',' (modified base64 uses ',' for '/')
    const ALPHA: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHA[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHA[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHA[((n >> 6) & 0x3f) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHA[(n & 0x3f) as usize] as char);
        }
    }
    out
}

/// Build the base64 lookup table (const fn, computed at compile time).
/// `,` maps to 63 (modified base64 uses `,` instead of `/`).
const fn build_b64_table() -> [i8; 256] {
    let mut t = [-1i8; 256];
    let alpha = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut i = 0;
    while i < alpha.len() {
        t[alpha[i] as usize] = i as i8;
        i += 1;
    }
    t[b',' as usize] = 63; // modified base64
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- decode ----

    #[test]
    fn imap_utf7_ascii_passthrough() {
        assert_eq!(decode_imap_utf7("INBOX"), "INBOX");
        assert_eq!(decode_imap_utf7("Sent Messages"), "Sent Messages");
    }

    #[test]
    fn imap_utf7_chinese_passthrough() {
        // user passes a Chinese name directly (no & segment); it should pass
        // through verbatim without corrupting UTF-8
        assert_eq!(
            decode_imap_utf7("其他文件夹/Github通知"),
            "其他文件夹/Github通知"
        );
    }

    #[test]
    fn imap_utf7_ampersand_escape() {
        // &- means a literal &
        assert_eq!(decode_imap_utf7("A&-B"), "A&B");
    }

    #[test]
    fn imap_utf7_single_chinese_char() {
        // "你" = U+4F60 → UTF-16BE 4F 60 → modified base64 "T2A"
        assert_eq!(decode_imap_utf7("&T2A-"), "你");
    }

    #[test]
    fn imap_utf7_mixed_chinese_and_ascii() {
        // "其他文件夹" prefix + "/Github"
        let decoded = decode_imap_utf7("&UXZO1mWHTvZZOQ-/Github&kBp35Q-");
        assert!(
            decoded.chars().any(|c| c as u32 > 127),
            "expected Chinese chars in: {decoded}"
        );
        assert!(decoded.contains("Github"));
    }

    #[test]
    fn imap_utf7_no_terminator_fallback() {
        // no terminating '-', emit as-is without panicking
        assert_eq!(decode_imap_utf7("test&abc"), "test&abc");
    }

    #[test]
    fn imap_utf7_roundtrip_known() {
        // "你好" → UTF-16BE 4F60 597D → base64: 4F 60 59 → 010011 110110 000001 011001 = T 2 B Z
        // remaining 7D → 011111 01(pad) = f Q → "T2BZfQ"
        assert_eq!(decode_imap_utf7("&T2BZfQ-"), "你好");
    }

    // ---- encode ----

    #[test]
    fn encode_ascii_passthrough() {
        assert_eq!(encode_imap_utf7("INBOX"), "INBOX");
        assert_eq!(encode_imap_utf7("Sent Messages"), "Sent Messages");
        assert_eq!(
            encode_imap_utf7("其他文件夹/From Me"),
            "&UXZO1mWHTvZZOQ-/From Me"
        );
    }

    #[test]
    fn encode_ampersand_escape() {
        assert_eq!(encode_imap_utf7("A&B"), "A&-B");
    }

    #[test]
    fn encode_single_and_multi_char() {
        assert_eq!(encode_imap_utf7("你"), "&T2A-");
        assert_eq!(encode_imap_utf7("你好"), "&T2BZfQ-");
    }

    #[test]
    fn encode_matches_known_production_names() {
        assert_eq!(
            encode_imap_utf7("其他文件夹/存档"),
            "&UXZO1mWHTvZZOQ-/&W1hoYw-"
        );
        assert_eq!(
            encode_imap_utf7("其他文件夹/Github通知"),
            "&UXZO1mWHTvZZOQ-/Github&kBp35Q-"
        );
    }

    /// Raw names observed in a real `folder_state` table — the encoder must
    /// reproduce each byte-for-byte, otherwise canonicalization would rename
    /// folders the server already named correctly.
    #[test]
    fn encode_roundtrips_production_raw_names() {
        let raw = [
            "&UXZO1mWHTvZZOQ-/&W1hoYw-",
            "&UXZO1mWHTvZZOQ-/&X1JoYw-",
            "&UXZO1mWHTvZZOQ-/&X65PF5T2iEyNJlNV-",
            "&UXZO1mWHTvZZOQ-/&X66Pb5Aad+U-",
            "&UXZO1mWHTvZZOQ-/&gX6Lr06RkBp35Q-",
            "&UXZO1mWHTvZZOQ-/12306&kBp35Q-",
            "&UXZO1mWHTvZZOQ-/163",
            "&UXZO1mWHTvZZOQ-/Agent&kBp35Q-",
            "&UXZO1mWHTvZZOQ-/Cloudcone&kBp35Q-",
            "&UXZO1mWHTvZZOQ-/Cloudflare&kBp35Q-",
            "&UXZO1mWHTvZZOQ-/From Me",
            "&UXZO1mWHTvZZOQ-/Github&kBp35Q-",
            "&UXZO1mWHTvZZOQ-/Google&kBp35Q-",
            "&UXZO1mWHTvZZOQ-/QQ&kK57sZAad+U-",
            "&UXZO1mWHTvZZOQ-/QQ&kK5O9ouilgU-",
            "&UXZO1mWHTvZZOQ-/Steam&kBp35Q-",
            "&UXZO1mWHTvZZOQ-/Vercel",
            "&UXZO1mWHTvZZOQ-/ZJU",
            "&UXZO1mWHTvZZOQ-/epic&ZTZjbg-",
            "&UXZO1mWHTvZZOQ-/python weekly",
            "&UXZO1mWHTvZZOQ-/relay",
            "INBOX",
            "Sent Messages",
            "Drafts",
            "Deleted Messages",
            "Junk",
        ];
        for name in raw {
            assert_eq!(
                encode_imap_utf7(&decode_imap_utf7(name)),
                name,
                "round-trip changed the raw name {name}"
            );
        }
    }

    // ---- canonical_folder_key ----

    #[test]
    fn canonical_key_maps_display_name_to_raw() {
        assert_eq!(
            canonical_folder_key("其他文件夹/存档"),
            "&UXZO1mWHTvZZOQ-/&W1hoYw-"
        );
        assert_eq!(
            canonical_folder_key("其他文件夹/Github通知"),
            "&UXZO1mWHTvZZOQ-/Github&kBp35Q-"
        );
    }

    #[test]
    fn canonical_key_keeps_raw_name_unchanged() {
        assert_eq!(
            canonical_folder_key("&UXZO1mWHTvZZOQ-/&W1hoYw-"),
            "&UXZO1mWHTvZZOQ-/&W1hoYw-"
        );
        assert_eq!(canonical_folder_key("Sent Messages"), "Sent Messages");
    }

    #[test]
    fn canonical_key_is_idempotent() {
        for name in [
            "其他文件夹/存档",
            "&UXZO1mWHTvZZOQ-/&W1hoYw-",
            "INBOX",
            "其他文件夹/From Me",
        ] {
            let once = canonical_folder_key(name);
            assert_eq!(canonical_folder_key(&once), once, "not idempotent: {name}");
        }
    }

    #[test]
    fn canonical_key_folds_inbox_case() {
        // RFC 3501: INBOX is case-insensitive — one physical folder, one key.
        assert_eq!(canonical_folder_key("inbox"), "INBOX");
        assert_eq!(canonical_folder_key("InBox"), "INBOX");
    }
}
