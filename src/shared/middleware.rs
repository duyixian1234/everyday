//! Middleware stack (P5, [F012](../../docs/adr/F012-architecture-deepening-phase.md)).
//!
//! Cross-cutting concerns (logging, timing, future metrics/retry) live in a
//! small middleware layer between the dispatcher and the modules, instead of
//! being duplicated inside each `execute`. The interface is deliberately
//! minimal — `before` / `after` / `on_error` — with the hidden behavior
//! (formatting, timing) contained in each middleware.
//!
//! Default stack: [`LoggingMiddleware`], enabled in `main.rs`. Modules are
//! completely unaware of middleware; disabling it leaves dispatch untouched.

use std::time::{Duration, Instant};

use crate::error::{AgentError, Result};
use crate::output::Output;
use crate::shared::request_context::RequestContext;

/// One middleware in the dispatch chain.
///
/// Hooks run in order for `before`; `after` runs (in reverse order) after a
/// dispatch returns, and `on_error` runs only when the dispatch failed. The
/// per-request `ctx` is passed explicitly so middleware never touches global
/// state (see [F013](../../docs/adr/F013-request-context-explicit-parameter.md)).
pub trait Middleware: Send + Sync {
    /// Called once before the module action executes.
    fn before(&self, _ctx: &RequestContext, _module: &str, _action: &str) {}

    /// Called after dispatch completes, with the outcome and elapsed time.
    fn after(
        &self,
        _ctx: &RequestContext,
        _module: &str,
        _action: &str,
        _result: &Result<Output>,
        _elapsed: Duration,
    ) {
    }

    /// Called only when dispatch failed.
    fn on_error(&self, _ctx: &RequestContext, _module: &str, _action: &str, _error: &AgentError) {}
}

/// Default middleware: emits an info-level tracing event per dispatch. The
/// subscriber renders it to stderr (never stdout, which is reserved for
/// command output) — compact text or a structured `_log` JSON line depending
/// on render mode. Events are silenced by the default WARN level; `-v`
/// (INFO) restores them. Stays in the stack unconditionally — level gating
/// is the subscriber's job, not the middleware's.
///
/// Field set is the R001 contract (`request_id` / `caller` / `module` /
/// `action`, plus `elapsed_ms` or `message`). These events are emitted
/// field-only, deliberately WITHOUT a format string: tracing's implicit
/// `message` field would collide with the explicit `message` payload of the
/// error path.
pub struct LoggingMiddleware;

impl Middleware for LoggingMiddleware {
    fn before(&self, ctx: &RequestContext, module: &str, action: &str) {
        tracing::info!(
            target: "everyday",
            _log = "start",
            request_id = %ctx.request_id,
            caller = %ctx.caller,
            module = %module,
            action = %action,
        );
    }

    fn after(
        &self,
        ctx: &RequestContext,
        module: &str,
        action: &str,
        result: &Result<Output>,
        elapsed: Duration,
    ) {
        // `as_millis()` is u128, but tracing's Visit has no u128 slot and the
        // JSON contract renders elapsed_ms as a number — narrowing to u64 is
        // lossless for any realistic duration (< 5.8e11 years).
        let elapsed_ms = elapsed.as_millis() as u64;
        let kind = if result.is_ok() { "ok" } else { "error" };
        tracing::info!(
            target: "everyday",
            _log = %kind,
            request_id = %ctx.request_id,
            caller = %ctx.caller,
            module = %module,
            action = %action,
            elapsed_ms = elapsed_ms,
        );
    }

    fn on_error(&self, ctx: &RequestContext, module: &str, action: &str, error: &AgentError) {
        tracing::info!(
            target: "everyday",
            _log = "error_detail",
            request_id = %ctx.request_id,
            caller = %ctx.caller,
            module = %module,
            action = %action,
            message = %error.message(),
        );
    }
}

/// Convenience: run a dispatch through a middleware chain.
///
/// `before` runs in order; on success/error `after` runs in reverse order;
/// `on_error` runs (reverse order) only on failure. Elapsed time spans the
/// whole chain. `module_name` is the registry key (e.g. `"mail"`) used in
/// logs, distinct from `module.description()`. `ctx` is the per-request
/// context built by the dispatcher and forwarded to every hook and to the
/// module's `execute`.
pub async fn run_with_middleware(
    middleware: &[Box<dyn Middleware>],
    ctx: &RequestContext,
    module_name: &str,
    module: &dyn crate::modules::Executor,
    action: &str,
    args: &[String],
) -> Result<Output> {
    let started = Instant::now();
    for m in middleware {
        m.before(ctx, module_name, action);
    }

    let result = module.execute(action, args, ctx).await;

    let elapsed = started.elapsed();
    if let Err(e) = &result {
        for m in middleware.iter().rev() {
            m.on_error(ctx, module_name, action, e);
        }
    }
    for m in middleware.iter().rev() {
        m.after(ctx, module_name, action, &result, elapsed);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::{Executor, ModuleArgSpec};
    use async_trait::async_trait;

    fn test_ctx() -> RequestContext {
        RequestContext::cli("test-req".into())
    }

    struct EchoModule;
    #[async_trait]
    impl Executor for EchoModule {
        fn description(&self) -> &'static str {
            "echo"
        }
        fn module_arg_spec(&self) -> ModuleArgSpec {
            ModuleArgSpec {
                name: "echo",
                description: "echo",
                actions: &[],
            }
        }
        async fn execute(
            &self,
            _action: &str,
            _args: &[String],
            _ctx: &RequestContext,
        ) -> Result<Output> {
            Ok(Output::text("ok"))
        }
    }

    /// Records hook invocations into a shared log; the test asserts order.
    #[derive(Default)]
    struct RecordingMiddleware {
        log: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl Middleware for RecordingMiddleware {
        fn before(&self, _ctx: &RequestContext, module: &str, action: &str) {
            self.log
                .lock()
                .unwrap()
                .push(format!("before:{module}:{action}"));
        }
        fn after(
            &self,
            _ctx: &RequestContext,
            _m: &str,
            _a: &str,
            _r: &Result<Output>,
            _e: Duration,
        ) {
            self.log.lock().unwrap().push("after".into());
        }
        fn on_error(&self, _ctx: &RequestContext, _m: &str, _a: &str, _e: &AgentError) {
            self.log.lock().unwrap().push("on_error".into());
        }
    }

    #[tokio::test]
    async fn hooks_run_in_order_and_pass_through() {
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let stack: Vec<Box<dyn Middleware>> =
            vec![Box::new(RecordingMiddleware { log: log.clone() })];
        let module: Box<dyn Executor> = Box::new(EchoModule);
        let out = run_with_middleware(&stack, &test_ctx(), "echo", module.as_ref(), "echo", &[])
            .await
            .unwrap();
        assert_eq!(out.render(crate::output::RenderMode::Text), "ok");
        let calls = log.lock().unwrap();
        assert_eq!(
            calls.as_slice(),
            &["before:echo:echo", "after"] // after runs in reverse order
        );
    }

    #[tokio::test]
    async fn on_error_runs_only_on_failure() {
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let stack: Vec<Box<dyn Middleware>> =
            vec![Box::new(RecordingMiddleware { log: log.clone() })];

        struct FailModule;
        #[async_trait]
        impl Executor for FailModule {
            fn description(&self) -> &'static str {
                "fail"
            }
            fn module_arg_spec(&self) -> ModuleArgSpec {
                ModuleArgSpec {
                    name: "fail",
                    description: "fail",
                    actions: &[],
                }
            }
            async fn execute(
                &self,
                _a: &str,
                _args: &[String],
                _ctx: &RequestContext,
            ) -> Result<Output> {
                Err(AgentError::Other("boom".into()))
            }
        }

        let module: Box<dyn Executor> = Box::new(FailModule);
        let result =
            run_with_middleware(&stack, &test_ctx(), "fail", module.as_ref(), "fail", &[]).await;
        assert!(result.is_err());
        let calls = log.lock().unwrap();
        assert_eq!(calls.as_slice(), &["before:fail:fail", "on_error", "after"]);
    }
}
