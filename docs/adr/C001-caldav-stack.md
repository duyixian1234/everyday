# ADR C001: CalDAV stack — libdav + icalendar + hyper-rustls (ring provider), skip DNS SRV bootstrap

**Status:** Accepted
**Date:** 2026-07-09

## Context

The `cal` module needs CalDAV access — discover calendars, list events, create / delete events. Three layers matter:

1. **WebDAV / CalDAV client.** Talk `PROPFIND`, `REPORT`, `MKCALENDAR`, etc.
2. **iCalendar parser/serializer.** RFC 5545.
3. **HTTP transport.** Async, with TLS.

Three early obstacles had to be solved before any of this could ship:

- **`rustls` crypto provider panic** when both `ring` and `aws-lc-rs` are present in the dep tree. Several crates default to different providers and the runtime refuses to pick.
- **`bootstrap_via_service_discovery`** — the textbook way to find a CalDAV server via DNS `SRV` / `TXT` records. **Unreliable from mainland China networks**; many providers don't publish the records.
- **QQ Mail's `/.well-known/caldav` returns 301** to a different host; the library must follow it (or be told to).

## Decision

### Library choices

- **`libdav`** — WebDAV / CalDAV client. Built on `hyper` 1.
- **`icalendar`** — RFC 5545 builder + parser. Builder chain requires `.done()`.
- **`hyper` 1 + `hyper-rustls` 0.27** — HTTP transport.
  - **`ring`** as the rustls crypto provider. `webpki-tokio` for cert verification.
- **`tower-http`** — `AddAuthorization` layer for Basic auth.

### `libdav` requires an HTTP client implementation

`libdav` doesn't bundle an HTTP client; we provide one via its `HttpClient` trait. The body type must be `String` (not `Bytes`):

```rust
let connector = tower::ServiceBuilder::new()
    .layer(tower_http::add_auth::AddAuthorization::basic(user, pass))
    .service(hyper_util::client::legacy::Client::builder(...).build::<_, String>(connector));

let webdav = WebDav::new_with_client(base_url, connector);
let caldav = CalDavClient::new(webdav); // skips bootstrap
```

### `rustls` crypto provider

Installed once at process start in `main.rs`:

```rust
rustls::crypto::ring::default_provider()
    .install_default()
    .ok(); // ignore AlreadyInstalled; let _ is acceptable
```

This avoids the runtime panic when multiple provider crates coexist.

### Skip `bootstrap_via_service_discovery`

`CalDavClient::new(...)` is used instead of `CalDavClient::new_via_service_discovery(...)`. Discovery relies on DNS `SRV` / `TXT` records that are unreliable on domestic networks and frequently absent on Chinese providers (QQ Mail, NetEase).

We **manually** call `find_context_path` (well-known redirect chain, up to 5 hops) and explicitly cover `webdav.base_url` when the final hop points elsewhere (QQ Mail's 301 case). The base URL override is a public field on `webdav`, so we just set it after the probe.

### Keyring convention

Same shape as mail ([M001](M001-imap-stack.md), [F002](F002-multi-account-keyring.md)):

- Service: `everyday/cal/<account>`.
- Empty-password keyring entry → don't bother logging in; return `Auth` early.
- Login validates that the password is non-empty before any network round-trip.

## Alternatives considered

### Use `caldav-rs` or `rustcal`

- Both exist but are less maintained than `libdav` and depend on different HTTP stacks.
- Rejected: `libdav` had the cleanest integration with the project's existing `hyper` / `rustls` setup.

### DNS-SRV-driven discovery

- The textbook approach per RFC 6764.
- Unreliable in mainland China; explicitly not used.
- Rejected.

### `aws-lc-rs` as the crypto provider

- Faster on some platforms but adds an extra C build dependency.
- Rejected: `ring` is already an indirect dep via `hyper-rustls`; switching adds friction.

### Implement iCalendar parsing by hand

- Rejected: `icalendar` is mature; the `.done()` builder chain is the only ergonomic wart.

## Consequences

- `rustls::crypto::ring::default_provider().install_default()` at `main.rs` startup is mandatory. Removing it reintroduces the dual-provider panic.
- The `libdav` `HttpClient` body type is fixed at `String` — switching to `Bytes` requires a trait bound adjustment on the connector service.
- QQ Mail accounts work because we explicitly re-point `base_url` after the well-known probe.
- The manual discovery code is `cal.login`'s only non-trivial network step; the rest of the module is straightforward `caldav.request(...)` calls.
- icalendar's CRLF output is what we write to the server — no extra normalization needed.
- `NaiveDateTime::and_utc()` returns `DateTime<Utc>` (not `Option`) — used directly in event start/end comparisons.

## Cross-references

- The full-pull + local-filter strategy that uses this stack: [C002](C002-full-pull-local-filter.md).
- The Timeline event mapping for `cal`: [L002](L002-calendar-window-refresh.md), [L007](L007-notion-ops-log.md).
- The shared Notion-style account + keyring convention: [F002](F002-multi-account-keyring.md).