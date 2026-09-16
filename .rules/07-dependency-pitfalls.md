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
- **Env-credential fallback (opt-in, [R020](../docs/adr/R020-env-credential-fallback.md)):**
  when the backend is unavailable, credentials may instead come from
  `EVERYDAY_<MODULE>_<ACCOUNT>_PASSWORD` **if** the user opted in
  (`[auth] env_credentials = true` or `EVERYDAY_ENV_CREDENTIALS=1`). Reads go
  keyring → env → error; `login` still always writes the keyring.

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

## `Notion` (removed)

The Notion API client dependency was removed in v0.13.0
([R019](../docs/adr/R019-remove-notion-provider.md)); its API quirks and error
handling notes below applied to the then-current code and no longer apply.

## Platform and toolchain traps

Non-obvious errors whose fix is mechanical. Entries that turned out to be *decisions*
were promoted to ADRs instead — linked where they exist.

- **Windows: cross-target typecheck fails without a Linux linker.** `cargo check
  --target x86_64-unknown-linux-gnu` needs `x86_64-linux-gnu-gcc` for `ring`'s build
  script. Keep Unix-only code under `#[cfg]` unit tests / CI coverage; do Windows
  full-test + `--all-targets` clippy locally.
- **Windows: `task run` output must go through the OS handle.** Rust std's UTF-8
  validation panics on a GBK console (e.g. `ipconfig`). Write via `WriteFile` and let
  the console code page render it ([F017](../docs/adr/F017-task-module.md)); captured
  records are decoded UTF-8-first with a GBK fallback, so stored text stays lossy.
- **Windows CI: `tasklist` CSV quotes PIDs.** Match the quoted PID field rather than
  parsing raw columns when testing liveness ([F016](../docs/adr/F016-daemon-sync-scheduler.md)).
- **`Drop` + `tokio::spawn` panics after runtime shutdown.** Probe
  `tokio::runtime::Handle::try_current()` first; if there is no handle, release the
  resource synchronously ([R003](../docs/adr/R003-pool-guard-drop.md)).
- **`Local.from_local_datetime(&ndt).unwrap()` panics on DST boundaries.** Use
  `.earliest()` for the spring-forward gap and `.latest()` for the fall-back ambiguous
  hour ([R004](../docs/adr/R004-dst-boundary-dates.md)).
- **`config get/set` needs array indices.** `get_dotted`/`set_dotted` take a numeric
  segment branch that indexes `Value::as_array()`; `resize` extends as needed.
- **Rust 1.97 clippy `doc_lazy_continuation` / `doc_overindented_list_items` are
  deny-by-default.** A `///` list item continued on the next line must be indented two
  spaces. rustfmt does not reflow docs, so any doc edit requires a local clippy run.
- **`parse_simple_args` must not treat `-1` as a flag.** Single-dash tokens are always
  values; only `--XXX` is a flag ([R005](../docs/adr/R005-parse-simple-args.md)).

## Duplicate transitive versions (accepted, not actionable)

`cargo deny check bans` reports ~27 duplicate-version warnings. They are **warnings,
not failures** (`multiple-versions = "warn"` in `deny.toml`), and they are **upstream
major-version splits that cannot be resolved from `Cargo.toml`**. Do not "fix" them by
pinning — each was traced, and every old version comes from a transitive dependency:

| Duplicate | Old version comes from |
| --- | --- |
| `windows-sys` 0.45 + 0.52 + 0.61 | `jni 0.21` (→ `rustls-platform-verifier` → reqwest), `ring 0.17`, our own `0.61` |
| `thiserror` 1 + 2 | `async-imap 0.11`, `jni 0.21` |
| `base64` 0.22 + 0.23 | `async-imap`, `charset` (mailparse), `hyper-util` |
| `sha2` 0.10 + 0.11 | `sqlx-core 0.9` |
| `tower-http` 0.6 + 0.7 | `reqwest 0.13`'s internal use |
| `getrandom` 0.2 + 0.4 | `ring 0.17`, `redox_users` (dirs-sys) |
| `syn` 2 + 3, `darling`, `nom`, `hashbrown`, `digest`, `block-buffer`, `cpufeatures`, `crypto-common`, `event-listener`, `async-channel` | mixed build-time (proc-macro) vs runtime strata — a proc-macro's `syn 2` and a runtime `syn 3` cannot unify |

- **`windows-sys 0.45` cannot be removed.** It reaches us only via
  `rustls-platform-verifier 0.6.2` → `jni 0.21.1`, whose Windows dep is
  `windows-sys ^0.45.0`. `rustls-platform-verifier` is a **non-optional** dependency of
  reqwest's `rustls-no-provider` / `rustls` features (not something we opt into), and it
  requires `jni ^0.21`. `jni 0.22` dropped that pin (moved `windows-sys` to a
  dev-dependency), but we cannot reach 0.22 until upstream raises its `jni`
  requirement. `cargo update -p windows-sys@0.45.0` and `-p jni` both report
  "Locking 0 packages".
- **`reqwest` 0.13.1 is the ceiling while the `webpki-roots` feature is named.**
  reqwest **removed** that feature in 0.13.5, so `cargo update` refuses to move past
  0.13.1. Dropping the token *does* unlock 0.13.5 — but was measured to remove
  **zero** duplicates (still `windows-sys` 0.45/0.52/0.61, `tower-http` 0.6/0.7,
  `thiserror` 1/2), so it buys the upgrade and nothing else. `webpki-roots` is also a
  direct dependency and the root store for `email.rs` / `calendar.rs`, so the crate
  stays regardless.
- The v0.19.0 dependency sweep that introduced these splits is recorded in
  [docs/releases.md](../docs/releases.md) and
  [G001](../docs/adr/G001-quality-tools-suite.md).

## New crate policy

- Always check what the crate already gives you before reaching for a
  dependency. E.g. `keyring` for secrets — not `dirs` + `chmod 600`.
- If a crate does something surprising that touches 5+ lines of glue, file an
  ADR — not all surprises are decisions, but borderline cases belong in the
  index so future readers don't re-discover them.
