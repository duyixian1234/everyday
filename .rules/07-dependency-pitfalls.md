# 07-dependency-pitfalls.md — Known Crate & Toolchain Pitfalls

> Whenever a crate does something non-obvious and the fix is mechanical (a
> renamed method, a missing feature flag, a positional-arg trap), record it
> here as a one-liner. **Decision-class choices** still go to
> [docs/adr/](../docs/adr/).

## Rust toolchain

- `cargo` / `rustc` ≥ 1.96 for **edition 2024**. Edition 2024 tightens `unsafe`
  and `gen` semantics — this repo currently doesn't use either.
- A `[lints]` table may evolve; expect new warnings on every toolchain bump.

## `lettre 0.11` (mail / SMTP)

- Correct feature set: `tokio1-rustls-tls` + `smtp-transport` + `pool` + `builder`.
  There is **no** `imap-pool` feature — the pool lives in a sibling crate.
- Use `ContentType::TEXT_PLAIN` (not `TEXT_PLAIN_UTF_8` — that variant
  doesn't exist in 0.11).
- Async SMTP: `AsyncSmtpTransport::<Tokio1Executor>::relay(host)` for
  STARTTLS on port 587.

## `async-imap 0.9.x`

- Built on `futures::AsyncRead`. The `tokio-rustls` TLS stream is a tokio type;
  bridge with `tokio_util::compat()` → `Session<Compat<TlsStream<TcpStream>>>`.
- `Fetch::envelope()` is a **method** that returns `Option<&Envelope>`, not a
  field.
- The `Address` type lives in `imap_proto::Address`, not in `async_imap::types`.
  Fields are `Option<Cow<[u8]>>` — convert with `String::from_utf8_lossy`.
- `uid_search` returns `HashSet<Uid>` synchronously. Don't `try_collect` it.
- `uid_fetch` returns a `Stream` — `try_collect` is correct here.
- `Session::list(Option<&str>, Option<&str>)` — **both** args are optional.
- Folder names can be IMAP UTF-7 encoded. The codebase has a hand-written
  `decode_imap_utf7` (modified base64 + UTF-16BE); do not introduce a new
  dependency for it.
- IMAP `Client::login` returns a tuple `(Error, T)`. Pattern-match with
  destructuring.
- The actual `Address` type path is `async_imap::imap_proto::Address`.

## `keyring`

- `Entry::new(service, account)`; service = `everyday/<module>/<account>`,
  account = the upstream username / token's owning identifier.
- Empty-password outcome is a normal `Auth` — never panic.
- Missing backend (headless box without Secret Service) surfaces as
  `KeyringUnavailable`; modules then offer an interactive prompt as fallback.

## `toml`

- `toml::Value::is_bool()` — there is **no** `is_boolean()`.
- `toml::Value::try_from(&struct)` converts a struct into a `Value` for
  dotted-path edits.
- For `config get/set` array-index access: numeric segments in a dotted path
  index into `Value::as_array()`, and `resize` extends the array as needed.

## `mailparse`

- Construct a fake mail to decode an encoded-word header:
  `parse_mail("X-Decoded: <s>\r\n\r\n")` then `headers[0].get_value()`.
- `ParsedMail::headers` is `Vec<MailHeader>`.
- `MailHeaderMap` is a **trait** — use `&ParsedMail` and access `.headers`
  directly. Don't take a trait object as a parameter type.

## `output.rs` — formatting

- `format!("{s:<0$}", s, w)` — the `0$` refers to the first positional arg
  (`&str`), **not** the width. Manual `pad(s, w)` is the fix.

## `hyper` / `hyper-rustls`

- `hyper::Body` type for `build()` must be `String`, not `Bytes`.
- `rustls::crypto::ring::default_provider().install_default()` is installed
  once at `main()` entry. The return is `Result`; a repeat install is
  `Err(AlreadyInstalled)` — use `let _ = ...`.
- `http::Uri::host()` (returns `&str`), not `host_str()` — don't confuse with
  `url::Url`.

## `libdav` (CalDAV)

- `CalDavClient::new(webdav)` skips DNS-SRV bootstrap (unavailable in CN).
- `find_context_path` does well-known redirect (max 5 hops).
- `webdav.base_url` is a `pub` field — override it when the well-known
  response redirects.
- `caldav.request(...)` accepts `FindCalendars`, `GetCalendarResources`,
  `GetProperty`, `PutResource`, `Delete`.
- On QQ CalDAV (`/.well-known/caldav` → 301), override `base_url`.

## `icalendar`

- `Calendar::new().push(Event::new()...)` needs `.done()` at the end.
- `str::parse::<Calendar>()` to deserialize.
- `DatePerhapsTime::date_naive()` returns `NaiveDate`.
- Output is CRLF; `NaiveDateTime::and_utc()` returns `DateTime<Utc>` (not
  `Option`).

## `Notion`

- Base URL: `https://api.notion.com/v1`. Headers: `Notion-Version: 2022-06-28`
  and `Authorization: Bearer <token>`.
- 429 → back off using `Retry-After` (default 1s), once.
- 401 / 403 → `AgentError::Auth`. Other non-2xx → `AgentError::Network`.

## New crate policy

- Always check what the crate already gives you before reaching for a
  dependency. E.g. `keyring` for secrets — not `dirs` + `chmod 600`.
- If a crate does something surprising that touches 5+ lines of glue, file an
  ADR — not all surprises are decisions, but borderline cases belong in the
  index so future readers don't re-discover them.
