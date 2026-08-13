//! Daemon auto-sync module (ADR F016).
//!
//! `everyday daemon run` is a resident process — the only role allowed to
//! pull periodically. This file wires the CLI surface: the `[daemon]` config
//! handling, the `run` / `status` actions, and the `enabled` gate (t1); the
//! sync-cycle engine lives in [`cycle`] (t2); the state file / pid-liveness
//! / anti-reentry live in [`state`] (t3).
//!
//! The module holds the full `Arc<Config>` — it is a cross-module
//! orchestrator that reads every source section plus `[daemon]`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::config::Config;
use crate::error::{AgentError, Result};
use crate::modules::daemon::cycle::{CycleLoopOptions, CycleResult, run_cycle, run_cycles};
use crate::modules::daemon::state::{DaemonSources, DaemonState};
use crate::modules::{ActionArgSpec, Executor, ModuleArgSpec, Output};
use crate::shared::request_context::RequestContext;
use crate::util::args::parse_simple_args;

pub mod cycle;
pub mod state;

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
            "status" => self.status(args).await,
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

        // Anti-reentry: refuse to start when another instance's pid is alive
        // (t3). Stale state (running=true but pid dead) passes — the startup
        // write below overwrites it.
        state::check_reentry()?;

        let (once, sources) = parse_run_args(args, &self.config.daemon.sources);
        let interval = Duration::from_secs(self.config.daemon.interval_seconds);

        // Startup state write (t3): pid / running=true / started_at.
        let initial = DaemonState::initial(
            std::process::id(),
            self.config.daemon.enabled,
            self.config.daemon.interval_seconds,
        );
        state::write(&initial);

        if once {
            // One cycle, then render the summary to stdout (the command
            // result — R001). Full shape alignment with `timeline sync`
            // lands in t4; this is the working baseline.
            let result = run_cycle(&self.config, &sources).await;
            // Per-cycle + exit state in one write for --once (t3).
            let mut final_state = initial;
            final_state.update_cycle(DaemonSources::from_cycle(
                &result.timeline,
                &result.mail,
                &result.rss,
            ));
            final_state.mark_exit(result.ok());
            state::write(&final_state);
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

        // Shared state mutated by each cycle and read back on exit.
        let state_guard = Arc::new(Mutex::new(initial));
        let guard_for_loop = state_guard.clone();
        let config = self.config.clone();
        run_cycles(
            move |_| {
                let cfg = config.clone();
                let src = sources.clone();
                let guard = guard_for_loop.clone();
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
                    // Per-cycle state write (t3): last_cycle_at / cycles /
                    // last_cycle_ok / sources. Lock is held only across
                    // synchronous calls (never across an await point). A
                    // poisoned mutex means a panic happened mid-update; the
                    // state file is best-effort anyway, so keep writing.
                    {
                        let mut s = guard.lock().unwrap_or_else(|e| e.into_inner());
                        s.update_cycle(DaemonSources::from_cycle(
                            &result.timeline,
                            &result.mail,
                            &result.rss,
                        ));
                        state::write(&s);
                    }
                }
            },
            CycleLoopOptions {
                once: false,
                interval,
                shutdown,
            },
        )
        .await;

        // Exit state write (t3): running=false + exit_at/exit_ok.
        {
            let mut s = state_guard.lock().unwrap_or_else(|e| e.into_inner());
            s.mark_exit(true);
            state::write(&s);
        }

        // Reached only when the loop was cancelled (Ctrl+C) — exit normally.
        Ok(Output::text(""))
    }

    /// `daemon status [--json]` — read the state file, probe pid liveness,
    /// and report whether the daemon is actually running (t3).
    async fn status(&self, _args: &[String]) -> Result<Output> {
        let Some(mut state) = state::read()? else {
            // No state file — daemon never started (or state was deleted).
            if crate::util::json_mode::is_json() {
                return Ok(Output::Json(serde_json::Value::Null));
            }
            return Ok(Output::text("daemon: not running (no state file found)"));
        };

        // `running` in the file can be stale (crash / kill -9). The effective
        // status is pid-liveness; correct the in-memory copy for output.
        if state.running && !state.is_effectively_running() {
            state.running = false;
        }

        if crate::util::json_mode::is_json() {
            let value = serde_json::to_value(&state).map_err(|e| {
                AgentError::Other(format!("daemon status: serialization failed: {e}"))
            })?;
            Ok(Output::Json(value))
        } else {
            Ok(render_status(&state))
        }
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
/// object. (The state file stores the same per-source data in its own schema,
/// see [`state`].)
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

/// Render the daemon status as human-readable text (t3).
fn render_status(state: &DaemonState) -> Output {
    let status = if state.running { "running" } else { "stopped" };
    let mut lines: Vec<String> = vec![format!("Status: {status}")];
    lines.push(format!("PID: {}", state.pid));
    lines.push(format!("Enabled: {}", state.enabled));
    lines.push(format!("Interval: {}s", state.interval_seconds));
    if let Some(t) = &state.started_at {
        lines.push(format!("Started: {}", t.to_rfc3339()));
    }
    if let Some(t) = &state.last_cycle_at {
        lines.push(format!(
            "Last cycle: {} ({})",
            t.to_rfc3339(),
            ok_label(state.last_cycle_ok)
        ));
    }
    lines.push(format!("Cycles: {}", state.cycles));
    if let Some(t) = &state.exit_at {
        lines.push(format!(
            "Exit: {} ({})",
            t.to_rfc3339(),
            ok_label(state.exit_ok)
        ));
    }

    let mut source_lines: Vec<String> = Vec::new();
    if let Some(tl) = &state.sources.timeline {
        let mut line = format!(
            "  timeline: {} ({} events",
            ok_label(Some(tl.ok)),
            tl.events
        );
        if let Some(e) = &tl.error {
            line.push_str(&format!(", error: {e}"));
        }
        line.push(')');
        source_lines.push(line);
    }
    if let Some(m) = &state.sources.mail {
        let mut line = format!(
            "  mail: {} ({} folders, {} envelopes",
            ok_label(Some(m.ok)),
            m.folders,
            m.envelopes
        );
        if let Some(e) = &m.error {
            line.push_str(&format!(", error: {e}"));
        }
        line.push(')');
        source_lines.push(line);
    }
    if let Some(r) = &state.sources.rss {
        let mut line = format!("  rss: {} ({} items", ok_label(Some(r.ok)), r.items);
        if let Some(e) = &r.error {
            line.push_str(&format!(", error: {e}"));
        }
        line.push(')');
        source_lines.push(line);
    }
    if !source_lines.is_empty() {
        lines.push("Sources:".into());
        lines.extend(source_lines);
    }

    Output::text(lines.join("\n"))
}

/// Human label for an optional success flag: `ok` / `failed` / `unknown`.
fn ok_label(ok: Option<bool>) -> &'static str {
    match ok {
        Some(true) => "ok",
        Some(false) => "failed",
        None => "unknown",
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

    // ── render_status ──

    fn sample_state(running: bool) -> DaemonState {
        use crate::modules::daemon::state::{
            DaemonSources, MailSourceState, RssSourceState, TimelineSourceState,
        };
        use chrono::{DateTime, Utc};
        let ts = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        DaemonState {
            pid: 12345,
            running,
            enabled: true,
            interval_seconds: 900,
            started_at: Some(ts("2026-08-13T23:00:00Z")),
            last_cycle_at: Some(ts("2026-08-13T23:15:00Z")),
            cycles: 3,
            last_cycle_ok: Some(true),
            exit_at: if running {
                None
            } else {
                Some(ts("2026-08-13T23:20:00Z"))
            },
            exit_ok: if running { None } else { Some(true) },
            sources: DaemonSources {
                timeline: Some(TimelineSourceState {
                    ok: true,
                    events: 12,
                    error: None,
                }),
                mail: Some(MailSourceState {
                    ok: true,
                    folders: 8,
                    envelopes: 34,
                    error: None,
                }),
                rss: Some(RssSourceState {
                    ok: true,
                    items: 5,
                    error: None,
                }),
            },
        }
    }

    #[test]
    fn render_status_running_contains_core_fields() {
        let out = render_status(&sample_state(true));
        let text = match out {
            Output::Text(t) => t,
            other => panic!("expected text output, got {other:?}"),
        };
        assert!(text.contains("Status: running"), "{text}");
        assert!(text.contains("PID: 12345"), "{text}");
        assert!(text.contains("Cycles: 3"), "{text}");
        assert!(text.contains("timeline: ok (12 events"), "{text}");
        assert!(text.contains("mail: ok (8 folders, 34 envelopes"), "{text}");
        assert!(text.contains("rss: ok (5 items"), "{text}");
        assert!(!text.contains("Exit:"), "{text}");
    }

    #[test]
    fn render_status_stopped_shows_exit_and_failed_source() {
        let mut state = sample_state(false);
        state.sources.mail = Some(crate::modules::daemon::state::MailSourceState {
            ok: false,
            folders: 0,
            envelopes: 0,
            error: Some("connection refused".into()),
        });
        let out = render_status(&state);
        let text = match out {
            Output::Text(t) => t,
            other => panic!("expected text output, got {other:?}"),
        };
        assert!(text.contains("Status: stopped"), "{text}");
        assert!(
            text.contains("Exit: 2026-08-13T23:20:00+00:00 (ok)"),
            "{text}"
        );
        assert!(
            text.contains("mail: failed (0 folders, 0 envelopes, error: connection refused)"),
            "{text}"
        );
    }

    #[test]
    fn render_status_empty_state_is_concise() {
        let state = DaemonState::default();
        let out = render_status(&state);
        let text = match out {
            Output::Text(t) => t,
            other => panic!("expected text output, got {other:?}"),
        };
        assert!(text.contains("Status: stopped"), "{text}");
        assert!(text.contains("PID: 0"), "{text}");
    }
}
