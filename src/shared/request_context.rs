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

use std::sync::OnceLock;
use std::time::Instant;

/// Snapshot of the current request's context.
///
/// `request_id` is generated once per request; `deadline` is `None` for
/// interactive CLI (REPL/API layers may set one); `caller` names the
/// front-end that initiated the request.
///
/// The `exit_code` slot is the one mutable policy channel: a module may
/// announce an explicit process exit code for a CLI invocation (e.g. `task
/// run` mirrors the child's status). It is read at the host boundary
/// (`finalize`) and ignored by front-ends without a process exit (MCP). It
/// uses a `OnceLock` so the context keeps flowing as `&` (and stays `Send +
/// Sync`) through the middleware chain and `Executor::execute` ([R023]). See
/// docs/adr/R023-exit-code-on-request-context.md.
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
    /// Explicit process exit code announced by the module; unset = default.
    exit_code: OnceLock<i32>,
}

impl RequestContext {
    /// A fresh context for a CLI invocation (no deadline).
    pub fn cli(request_id: String) -> Self {
        Self {
            request_id,
            deadline: None,
            caller: "cli",
            exit_code: OnceLock::new(),
        }
    }

    /// A fresh context for an MCP tool invocation.
    pub fn mcp(request_id: String) -> Self {
        Self {
            request_id,
            deadline: None,
            caller: "mcp",
            exit_code: OnceLock::new(),
        }
    }

    /// The explicit process exit code announced by the module, if any.
    #[allow(dead_code)] // exercised by unit tests; the boundary uses effective_exit_code
    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code.get().copied()
    }

    /// The exit code a CLI invocation should use: the announced code, or the
    /// default `0` for a plain successful output.
    pub fn effective_exit_code(&self) -> i32 {
        self.exit_code.get().copied().unwrap_or(0)
    }

    /// Announce an explicit process exit code for this invocation.
    ///
    /// Only a CLI front-end consumes this; front-ends without a process exit
    /// (MCP) ignore it, so a module may set it unconditionally. A second
    /// announcement is ignored (first wins).
    pub fn set_exit_code(&self, code: i32) {
        let _ = self.exit_code.set(code);
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
    fn exit_code_defaults_to_none() {
        let ctx = RequestContext::cli("req-1".into());
        assert_eq!(ctx.exit_code(), None);
        assert_eq!(ctx.effective_exit_code(), 0);
    }

    #[test]
    fn exit_code_can_be_set_and_read() {
        let ctx = RequestContext::cli("req-1".into());
        ctx.set_exit_code(124);
        assert_eq!(ctx.exit_code(), Some(124));
        assert_eq!(ctx.effective_exit_code(), 124);
    }

    #[test]
    fn exit_code_mcp_defaults_to_none() {
        let ctx = RequestContext::mcp("req-2".into());
        assert_eq!(ctx.exit_code(), None);
        assert_eq!(ctx.effective_exit_code(), 0);
    }

    #[test]
    fn generated_id_is_unique_and_prefixed() {
        let a = generate_request_id();
        let b = generate_request_id();
        assert!(a.starts_with("cli-"));
        assert_ne!(a, b);
    }
}
