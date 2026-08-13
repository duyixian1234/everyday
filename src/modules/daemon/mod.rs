//! Daemon auto-sync module (ADR F016).
//!
//! `everyday daemon run` is a resident process — the only role allowed to
//! pull periodically. This file wires the CLI surface (t1): the `[daemon]`
//! config handling, the `run` / `status` actions, and the `enabled` gate.
//! The sync-cycle engine lives in [`cycle`] (t2); the state file / status
//! output land in t3 (status still returns a placeholder until then).
//!
//! The module holds the full `Arc<Config>` — it is a cross-module
//! orchestrator that reads every source section plus `[daemon]`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::config::Config;
use crate::error::{AgentError, Result};
use crate::modules::daemon::cycle::{CycleLoopOptions, CycleResult, run_cycle, run_cycles};
use crate::modules::{ActionArgSpec, Executor, ModuleArgSpec, Output};
use crate::shared::request_context::RequestContext;
use crate::util::args::parse_simple_args;

pub mod cycle;

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
        args: &[String],
        _ctx: &RequestContext,
    ) -> Result<Output> {
        match action {
            "run" => self.run(args).await,
            "status" => Err(AgentError::Other(
                "daemon status: not implemented yet (t3: state file)".into(),
            )),
            other => Err(AgentError::UnknownAction(format!("daemon {other}"))),
        }
    }
}

impl DaemonModule {
    /// `daemon run [--once] [--sources mail,rss]`.
    async fn run(&self, args: &[String]) -> Result<Output> {
        // The `enabled` switch is the "should this process be resident" gate:
        // a service-manager restart loop must not spin an empty process (ADR
        // F016). Checked before any cycle work so a disabled daemon fails fast.
        if !self.config.daemon.enabled {
            return Err(AgentError::Config(
                "daemon disabled in config ([daemon] enabled = false)".into(),
            ));
        }

        let (once, sources) = parse_run_args(args, &self.config.daemon.sources);
        let interval = Duration::from_secs(self.config.daemon.interval_seconds);
        if once {
            // One cycle, then render the summary to stdout (the command
            // result — R001). Full shape alignment with `timeline sync`
            // lands in t4; this is the working baseline.
            let result = run_cycle(&self.config, &sources).await;
            return Ok(render_cycle(&result));
        }

        // Resident mode: Ctrl+C cancels the loop (full signal handling and
        // state-file finalization land in t5). stdout stays silent (R001).
        let shutdown = CancellationToken::new();
        {
            let sig = shutdown.clone();
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                sig.cancel();
            });
        }
        let config = self.config.clone();
        run_cycles(
            move |_| {
                let cfg = config.clone();
                let src = sources.clone();
                async move {
                    let result = run_cycle(&cfg, &src).await;
                    tracing::info!(
                        target: "everyday",
                        _log = "cycle_completed",
                        ok = %result.ok(),
                        timeline_events = result.timeline.as_ref().map(|a| a.events).unwrap_or(0),
                        mail_folders = result.mail.as_ref().map(|a| a.folders).unwrap_or(0),
                        mail_envelopes = result.mail.as_ref().map(|a| a.envelopes).unwrap_or(0),
                        rss_items = result.rss.as_ref().map(|a| a.items).unwrap_or(0),
                    );
                }
            },
            CycleLoopOptions {
                once: false,
                interval,
                shutdown,
            },
        )
        .await;

        // Reached only when the loop was cancelled (Ctrl+C) — exit normally.
        Ok(Output::text(""))
    }
}

/// Parse `daemon run` flags: `--once` (bool) and `--sources` (comma list,
/// overriding the config whitelist; empty string → empty list = all sources).
fn parse_run_args(args: &[String], config_sources: &[String]) -> (bool, Vec<String>) {
    let (flags, _) = parse_simple_args(args);
    let once = flags.contains_key("once");
    let sources: Vec<String> = match flags.get("sources") {
        Some(s) => s
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
        None => config_sources.to_vec(),
    };
    (once, sources)
}

/// Render one cycle's result (the `--once` command result; R001). Text mode
/// prints one line per executed action; JSON mode emits the structured
/// object (t3 reuses the same shape for the state file).
fn render_cycle(result: &CycleResult) -> Output {
    let action = |a: &Option<crate::modules::daemon::cycle::ActionResult>| match a {
        None => serde_json::Value::Null,
        Some(a) => serde_json::json!({
            "ok": a.ok,
            "events": a.events,
            "folders": a.folders,
            "envelopes": a.envelopes,
            "items": a.items,
            "error": a.error,
        }),
    };
    if crate::util::json_mode::is_json() {
        Output::Json(serde_json::json!({
            "started_at": result.started_at.to_rfc3339(),
            "timeline": action(&result.timeline),
            "mail": action(&result.mail),
            "rss": action(&result.rss),
        }))
    } else {
        let mut lines: Vec<String> = Vec::new();
        for (label, a) in [
            ("timeline", &result.timeline),
            ("mail", &result.mail),
            ("rss", &result.rss),
        ] {
            match a {
                None => {}
                Some(a) if a.ok => {
                    let detail = match label {
                        "timeline" => format!("{} events", a.events),
                        "mail" => format!("{} folders, {} envelopes", a.folders, a.envelopes),
                        _ => format!("{} items", a.items),
                    };
                    lines.push(format!("{label}: {detail}"));
                }
                Some(a) => lines.push(format!(
                    "{label}: failed: {}",
                    a.error.as_deref().unwrap_or("unknown error")
                )),
            }
        }
        Output::text(lines.join("\n"))
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

    #[test]
    fn parse_run_args_once_flag() {
        let (once, sources) = parse_run_args(&["--once".into()], &[]);
        assert!(once);
        assert!(sources.is_empty());
    }

    #[test]
    fn parse_run_args_sources_override() {
        let (once, sources) =
            parse_run_args(&["--sources".into(), "mail,rss".into()], &["cal".into()]);
        assert!(!once);
        assert_eq!(sources, vec!["mail", "rss"]);
    }

    #[test]
    fn parse_run_args_defaults_to_config_sources() {
        let (once, sources) = parse_run_args(&[], &["mail".into()]);
        assert!(!once);
        assert_eq!(sources, vec!["mail"]);
    }

    #[test]
    fn parse_run_args_sources_ignores_empty_tokens() {
        let (_, sources) = parse_run_args(
            &["--sources".into(), "mail,, rss ,".into()],
            &["cal".into()],
        );
        assert_eq!(sources, vec!["mail", "rss"]);
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
