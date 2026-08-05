//! Per-request context propagation (P4, [F012](../../docs/adr/F012-architecture-deepening-phase.md)).
//!
//! `RequestContext` carries `request_id`, an optional `deadline`, and the
//! `caller` (CLI / REPL / API layer) through the whole dispatch stack. In
//! v0.11 it is propagated **non-breakingly** via a thread-local set once by
//! `main.rs` per command — modules and middleware can read it without an
//! `execute` signature change. The explicit parameter form (breaking) is
//! deferred to v0.12.
//!
//! This mirrors the `--json` thread-local pattern ([R001](../../docs/adr/R001-thread-local-json-mode.md)):
//! one set site, many read sites, no threading through every call.

use std::cell::RefCell;
use std::time::Instant;

/// Immutable snapshot of the current request's context.
///
/// `request_id` is generated once per CLI invocation; `deadline` is `None`
/// for interactive CLI (REPL/API layers may set one); `caller` names the
/// front-end that initiated the request.
#[derive(Debug, Clone)]
pub struct RequestContext {
    /// Unique id for this command invocation (e.g. `cli-<nanos>-<pid>`).
    pub request_id: String,
    /// Optional absolute deadline; enforcement is the caller's job.
    #[allow(dead_code)] // consumed by deadline enforcement in v0.12 (P4)
    pub deadline: Option<Instant>,
    /// Front-end that initiated the request (`"cli"`, `"repl"`, `"api"`).
    #[allow(dead_code)] // consumed by tracing/permissions in v0.12 (P4)
    pub caller: &'static str,
}

impl RequestContext {
    /// A fresh context for a CLI invocation (no deadline).
    pub fn cli(request_id: String) -> Self {
        Self {
            request_id,
            deadline: None,
            caller: "cli",
        }
    }
}

thread_local! {
    /// Per-thread request context. `None` outside a dispatched request.
    static CURRENT: RefCell<Option<RequestContext>> = const { RefCell::new(None) };
}

/// Install the request context for the current thread (called once per
/// command by the dispatcher before dispatch).
pub fn set_request_context(ctx: RequestContext) {
    CURRENT.with(|c| *c.borrow_mut() = Some(ctx));
}

/// Clear the request context (called by the dispatcher after dispatch).
pub fn clear_request_context() {
    CURRENT.with(|c| *c.borrow_mut() = None);
}

/// The current request context, if one is installed.
pub fn current() -> Option<RequestContext> {
    CURRENT.with(|c| c.borrow().clone())
}

/// The current request id, if a context is installed.
pub fn request_id() -> Option<String> {
    current().map(|c| c.request_id)
}

/// Generate a process-unique request id: `cli-<unix_nanos>-<pid>`.
pub fn generate_request_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    format!("cli-{nanos}-{pid}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_roundtrip() {
        set_request_context(RequestContext::cli("req-1".into()));
        let ctx = current().unwrap();
        assert_eq!(ctx.request_id, "req-1");
        assert_eq!(ctx.caller, "cli");
        assert!(ctx.deadline.is_none());
        clear_request_context();
        assert!(current().is_none());
    }

    #[test]
    fn request_id_helper() {
        clear_request_context();
        assert!(request_id().is_none());
        set_request_context(RequestContext::cli("req-2".into()));
        assert_eq!(request_id().as_deref(), Some("req-2"));
        clear_request_context();
    }

    #[test]
    fn generated_id_is_unique_and_prefixed() {
        let a = generate_request_id();
        let b = generate_request_id();
        assert!(a.starts_with("cli-"));
        assert_ne!(a, b);
    }
}
