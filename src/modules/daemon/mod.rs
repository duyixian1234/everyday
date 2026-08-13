//! Daemon auto-sync module (ADR F016).
//!
//! `everyday daemon run` is a resident process — the only role allowed to
//! pull periodically. This ticket (t1, issue #11) wires the CLI surface:
//! the `[daemon]` config handling, the `run` / `status` actions, and the
//! `enabled` gate. The sync-cycle engine (t2) and the state file / status
//! output (t3) land in follow-up tickets; until then both actions return a
//! clear "not implemented" error.
//!
//! The module holds the full `Arc<Config>` — it is a cross-module
//! orchestrator that reads every source section plus `[daemon]`.

use std::sync::Arc;

use async_trait::async_trait;

use crate::config::Config;
use crate::error::{AgentError, Result};
use crate::modules::{ActionArgSpec, Executor, ModuleArgSpec, Output};
use crate::shared::request_context::RequestContext;

/// Resident auto-sync module (ADR F016).
pub struct DaemonModule {
    config: Arc<Config>,
}

impl DaemonModule {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Executor for DaemonModule {
    fn description(&self) -> &'static str {
        "Resident auto-sync daemon (periodic timeline/mail/rss pull, ADR F016)."
    }

    fn module_arg_spec(&self) -> ModuleArgSpec {
        static ACTIONS: &[ActionArgSpec] = &[
            cli_action!(
                "run",
                "前台常驻运行：启动立即同步一次，之后每 interval_seconds 一个同步周期",
                "everyday daemon run [--once] [--sources mail,rss]",
                &[
                    flag!(
                        "once",
                        "只跑一个同步周期后退出（同步汇总输出到 stdout）",
                        Bool
                    ),
                    flag!("sources", "覆盖 [daemon].sources 白名单（逗号分隔）"),
                ]
            ),
            cli_action!(
                "status",
                "查询 daemon 运行状态（运行中 / 上次周期 / 各源结果）",
                "everyday daemon status [--json]",
                &[]
            ),
        ];
        ModuleArgSpec {
            name: "daemon",
            description: self.description(),
            actions: ACTIONS,
        }
    }

    async fn execute(
        &self,
        action: &str,
        _args: &[String],
        _ctx: &RequestContext,
    ) -> Result<Output> {
        match action {
            "run" => {
                // The `enabled` switch is the "should this process be
                // resident" gate: a service-manager restart loop must not
                // spin an empty process (ADR F016). Checked before any
                // cycle work (t2) so a disabled daemon fails fast.
                if !self.config.daemon.enabled {
                    return Err(AgentError::Config(
                        "daemon disabled in config ([daemon] enabled = false)".into(),
                    ));
                }
                Err(AgentError::Other(
                    "daemon run: not implemented yet (t2: sync cycle)".into(),
                ))
            }
            "status" => Err(AgentError::Other(
                "daemon status: not implemented yet (t3: state file)".into(),
            )),
            other => Err(AgentError::UnknownAction(format!("daemon {other}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::request_context::RequestContext;

    fn daemon_module(enabled: bool) -> DaemonModule {
        let mut cfg = Config::default();
        cfg.daemon.enabled = enabled;
        DaemonModule::new(Arc::new(cfg))
    }

    #[tokio::test]
    async fn run_refuses_when_disabled() {
        let err = daemon_module(false)
            .execute("run", &[], &RequestContext::cli("t1".into()))
            .await
            .unwrap_err();
        assert_eq!(err.type_name(), "ConfigError");
        assert!(
            err.message().contains("disabled"),
            "message: {}",
            err.message()
        );
    }

    #[tokio::test]
    async fn run_placeholder_until_t2() {
        // t1 only wires the surface: with enabled=true the cycle engine (t2)
        // is not implemented yet, and that must surface as an explicit error.
        let err = daemon_module(true)
            .execute("run", &[], &RequestContext::cli("t1".into()))
            .await
            .unwrap_err();
        assert!(
            err.message().contains("not implemented"),
            "{}",
            err.message()
        );
    }

    #[tokio::test]
    async fn unknown_action_rejected() {
        let err = daemon_module(true)
            .execute("stop", &[], &RequestContext::cli("t1".into()))
            .await
            .unwrap_err();
        assert_eq!(err.type_name(), "UnknownAction");
    }
}
