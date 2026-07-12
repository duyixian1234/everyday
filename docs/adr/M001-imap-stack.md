# ADR M001: IMAP stack — async-imap + tokio-rustls compat bridge + custom IMAP UTF-7 decoder + lettre SMTP

**Status:** Accepted
**Date:** 2026-07-08

> **Update (2026-07-12):** Credential & `login` logic consolidated into the top-level `auth` module. This module's `login` subcommand is removed; `mail` now calls `auth::get_credential`. See [R013](R013-auth-module-consolidation.md) (and [R014](R014-auth-verify-opt-in.md) / [R015](R015-auth-credential-io.md)).

## Context

The `mail` module needs:

- An **IMAP client** that integrates with the project's tokio runtime and `rustls`-only TLS stack.
- An **SMTP client** for outbound mail.
- An **MIME / encoded-word decoder** for subject and address headers.
- A **TLS bridge** between `async-imap` (which speaks `futures::AsyncRead`) and `tokio-rustls` (which speaks `tokio`).

Three obstacles blocked off-the-shelf choices:

1. **`async-imap` is futures-based; `tokio-rustls` is tokio-based.** They do not interop directly. A bridge is required.
2. **No maintained `imap-pool` crate exists.** The `lettre` crate's `imap-pool` feature is documented but not present in 0.11. The pool had to be built locally — see [M002](M002-imap-connection-pool.md).
3. **IMAP folder names are UTF-7 encoded.** Mailboxes with Chinese names ("已发送", "草稿") are encoded as IMAP modified UTF-7 on the wire. `async-imap` does not decode this; we need a decoder that runs at folder-selection time.

## Decision

### Library choices

- **`async-imap` 0.9.7** — IMAP client.
- **`lettre` 0.11** — SMTP outbound, with the exact feature set: `tokio1-rustls-tls` + `smtp-transport` + `pool` + `builder`. `imap-pool` does not exist; see [M002](M002-imap-connection-pool.md) for the local replacement.
- **`mailparse`** — MIME parser; `ParsedMail::headers` is `Vec<MailHeader>`.
- **`keyring` + `rpassword`** — credentials and password prompts (see [F002](F002-multi-account-keyring.md)).
- **`dirs`** — `config_dir()` for cross-platform config paths.

### TLS bridge

`async-imap::Session<T>` expects `T: futures::AsyncRead + futures::AsyncWrite`. `tokio_rustls::TlsStream<tokio::net::TcpStream>` does not implement those traits.

Use `tokio_util::compat::TokioAsyncReadCompatExt::compat()`:

```rust
let tls_stream: tokio_rustls::client::TlsStream<tokio::net::TcpStream> = ...;
let imap_stream: tokio_util::compat::Compat<TlsStream<TcpStream>> = tls_stream.compat();
let session: async_imap::Session<Compat<TlsStream<TcpStream>>> = async_imap::Client::new(imap_stream).login(user, pass)?;
```

The `Compat` wrapper adapts both directions. This is a one-line bridge; the rest of the IMAP code is unchanged.

### Custom IMAP UTF-7 decoder

IMAP folder names on the wire use modified UTF-7 (RFC 3501 §5.1.3). For example "已发送" is encoded as `&TBL-kyfnZ-`. No first-party Rust crate handles this well.

- Implementation: hand-written modified base64 + UTF-16BE decoder, no extra dependency.
- `select_folder(account, name)` does **smart matching**: try the raw name first, fall back to the decoded form. This avoids mismatches when the server echoes UTF-7 in some responses and UTF-8 in others.

### IMAP API conventions (read directly from `async-imap` source)

- `Fetch::envelope()` is a method returning `Option<&Envelope>`.
- `Address` lives in `async_imap::imap_proto::Address` (not `async_imap::types`). Fields are `Option<Cow<[u8]>>` → `String::from_utf8_lossy` for display.
- `uid_search` returns `HashSet<Uid>` (not a stream). `uid_fetch` returns a stream — collect with `try_collect`.
- `Client::login` returns `(Error, T)` so the unauthenticated client is still recoverable on auth failure.
- `Session::list(Option<&str>, Option<&str>)` — both arguments are optional.

### MIME encoded-word headers

Subjects and names like `=?UTF-8?B?...?=` are decoded via `mailparse`:

```rust
let parsed = mailparse::parse_mail(&format!("X-Decoded: {header}\r\n\r\n"))?;
let decoded = parsed.headers.get(0).unwrap().get_value();
```

The wrapper is necessary because `mailparse` expects a full message, not a bare header.

### Keyring convention for mail

- Service: `everyday/mail/<account>`.
- Account: the IMAP login username.
- Password prompts run inside `tokio::task::spawn_blocking` because `rpassword::prompt_password` is blocking.

### SMTP via lettre

- `AsyncSmtpTransport::<Tokio1Executor>::relay(host)` — STARTTLS on 587.
- `ContentType::TEXT_PLAIN` (not `TEXT_PLAIN_UTF_8`).
- Same keyring entry as IMAP; SMTP credentials are usually the IMAP credentials.

## Alternatives considered

### IMAP via `imap` crate (sync)

- Pro: stable, mature.
- Con: synchronous on a `tokio` runtime means spawning a thread per operation.
- Rejected: latency overhead and thread management.

### `async-imap-proto` directly

- Lower-level; no `Client::login`, no `Session::select`, etc.
- Rejected: would re-implement what `async-imap` already wraps.

### `imap-codec` for UTF-7

- Pro: pure parser, no IMAP dependencies.
- Con: adds a dependency for one direction (decode). The encoding direction is rarely needed.
- Considered: could replace the hand-written decoder. Not yet swapped because the current decoder is small and tested.

### Use a third-party SMTP crate instead of `lettre`

- `lettre` is the de-facto Rust SMTP client; alternatives are niche.
- Rejected: no upside.

## Consequences

- The TLS bridge is one line of code; the rest of the IMAP integration is straightforward.
- The IMAP UTF-7 decoder is a small, well-tested piece of code — no extra crate to track.
- The connection pool built on top of this stack ([M002](M002-imap-connection-pool.md)) reuses the same TLS bridge and keyring convention.
- Subjects / addresses are decoded at the boundary so the rest of the codebase deals in UTF-8 strings.
- Direct dependency footprint stays small: `async-imap`, `lettre`, `mailparse`, `keyring`, `rpassword`, `tokio-util`.

## Cross-references

- The connection pool built on this stack: [M002](M002-imap-connection-pool.md).
- The envelope cache that uses the IMAP `UID FETCH (UID ENVELOPE FLAGS RFC822.SIZE)` shape: [M003](M003-envelope-cache.md), [M004](M004-uid-watermark-sync.md).
- How often the IMAP stack is exercised by `mail list`: [M005](M005-staleness-auto-sync.md).
- The thread-local JSON mode that drives both `--json` IMAP output and AOP log writes: [R001](R001-thread-local-json-mode.md), [L011](L011-aop-handles-output-text.md).