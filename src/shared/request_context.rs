//! Per-request context propagation (P4, [F012](../../docs/adr/F012-architecture-deepening-phase.md)).
//!
//! `RequestContext` carries `request_id`, an optional `deadline`, and the
//! `caller` (CLI / REPL / API layer). Since v0.12 it is passed **explicitly**
//! as a parameter through `Executor::execute` and the middleware stack (see
//! [F013](../../docs/adr/F013-request-context-explicit-parameter.md)) — this
//! replaced the v0.11 thread-local propagation, which could not serve
//! concurrent front-ends (REPL / API / batch tools).
//!
//! The dispatcher builds one context per request and passes the same `&` into
//! middleware hooks and the module's `execute`; consumers read fields off the
//! reference. There is no global state.

use std::time::Instant;

/// Immutable snapshot of the current request's context.
///
/// `request_id` is generated once per request; `deadline` is `None` for
/// interactive CLI (REPL/API layers may set one); `caller` names the
/// front-end that initiated the request.
#[derive(Debug, Clone)]
pub struct RequestContext {
    /// Unique id for this request (e.g. `cli-<nanos>-<pid>`).
    pub request_id: String,
    /// Optional absolute deadline; enforcement is the caller's job (a future
    /// deadline-enforcement middleware may consume it).
    #[allow(dead_code)] // deadline enforcement is future work (REPL/API callers may set one)
    pub deadline: Option<Instant>,
    /// Front-end that initiated the request (`"cli"`, `"repl"`, `"api"`).
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

    /// A fresh context for an MCP tool invocation.
    pub fn mcp(request_id: String) -> Self {
        Self {
            request_id,
            deadline: None,
            caller: "mcp",
        }
    }
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
    fn cli_defaults() {
        let ctx = RequestContext::cli("req-1".into());
        assert_eq!(ctx.request_id, "req-1");
        assert_eq!(ctx.caller, "cli");
        assert!(ctx.deadline.is_none());
    }

    #[test]
    fn generated_id_is_unique_and_prefixed() {
        let a = generate_request_id();
        let b = generate_request_id();
        assert!(a.starts_with("cli-"));
        assert_ne!(a, b);
    }
}
