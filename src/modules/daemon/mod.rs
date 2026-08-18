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
            // F017: `--once` includes one cron scheduler pass before the sync
            // summary. Scheduled task output is captured into task.db and
            // never written to daemon stdout.
            let task_store = crate::modules::task::store::TaskStore::open_default().await?;
            let task_pass = crate::modules::task::scheduler::run_due_tasks(
                &self.config,
                &task_store,
                chrono::Local::now(),
            )
            .await?;
            log_task_pass(task_pass);

            // One cycle, then render the summary to stdout (the command
            // result — R001). Full shape alignment with `timeline sync`
            // (t4). Per-cycle + exit state in one write for --once (t3);
            // a failed final write surfaces as `_error` + exit 1 (t4).
            let result = run_cycle(&self.config, &sources).await;
            // The cycle record goes to daemon.log in both modes (t4).
            log_cycle_completed(&result);
            let mut final_state = initial;
            final_state.update_cycle(DaemonSources::from_cycle(
                &result.timeline,
                &result.mail,
                &result.rss,
            ));
            final_state.mark_exit(result.ok());
            finalize_state(&final_state)?;
            return Ok(render_cycle(&result));
        }

        // Resident mode: any stop signal cancels the loop and funnels into
        // [`graceful_shutdown`] (t5). On Unix both SIGINT (Ctrl+C) and
        // SIGTERM (service managers) are wired; Windows has ctrl_c only
        // (tokio provides it cross-platform). stdout stays silent (R001).
        let shutdown = CancellationToken::new();
        {
            let sig = shutdown.clone();
            tokio::spawn(async move {
                #[cfg(unix)]
                {
                    let mut term =
                        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                            .expect("install SIGTERM handler");
                    tokio::select! {
                        _ = tokio::signal::ctrl_c() => {}
                        _ = term.recv() => {}
                    }
                }
                #[cfg(not(unix))]
                {
                    let _ = tokio::signal::ctrl_c().await;
                }
                sig.cancel();
            });
        }

        // Shared state mutated by each cycle and read back on exit.
        let state_guard = Arc::new(Mutex::new(initial));
        let guard_for_loop = state_guard.clone();
        let config = self.config.clone();
        let scheduler_shutdown = shutdown.clone();
        let scheduler_config = self.config.clone();
        let scheduler_handle = tokio::spawn(async move {
            let result =
                crate::modules::task::scheduler::run_loop(scheduler_config, scheduler_shutdown)
                    .await;
            if let Err(error) = &result {
                tracing::error!(
                    target: "everyday",
                    _error = "task_scheduler_failed",
                    message = %error.message(),
                );
            }
            result
        });
        run_cycles(
            move |_| {
                let cfg = config.clone();
                let src = sources.clone();
                let guard = guard_for_loop.clone();
                async move {
                    let result = run_cycle(&cfg, &src).await;
                    log_cycle_completed(&result);
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
                shutdown: shutdown.clone(),
            },
        )
        .await;
        shutdown.cancel();
        match scheduler_handle.await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {}
            Err(error) => tracing::error!(
                target: "everyday",
                _error = "task_scheduler_join_failed",
                message = %error,
            ),
        }

        // Reached only when a stop signal cancelled the loop — graceful
        // shutdown (t5): write the final state, then exit 0.
        graceful_shutdown(&state_guard)?;
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

/// Emit the per-cycle INFO record — the daemon.log sync record (t4). Emitted
/// by both `--once` and the resident loop so the file log always carries
/// "INFO 级同步记录" (ADR F016).
fn log_cycle_completed(result: &CycleResult) {
    tracing::info!(
        target: "everyday",
        _log = "cycle_completed",
        ok = result.ok(),
        timeline_events = result.timeline.as_ref().map(|a| a.events).unwrap_or(0),
        mail_folders = result.mail.as_ref().map(|a| a.folders).unwrap_or(0),
        mail_envelopes = result.mail.as_ref().map(|a| a.envelopes).unwrap_or(0),
        rss_items = result.rss.as_ref().map(|a| a.items).unwrap_or(0),
    );
}

fn log_task_pass(pass: crate::modules::task::scheduler::SchedulerPass) {
    tracing::info!(
        target: "everyday",
        _log = "task_scheduler_pass",
        scheduled = pass.scheduled,
        ran = pass.ran,
        failed = pass.failed,
    );
}

/// Render one cycle's result (the `--once` command result; R001), aligned
/// with the `timeline sync` output shape (t4): text opens with a summary
/// line then one line per executed action; JSON carries a top-level `ok`
/// summary plus per-source objects with only their relevant fields
/// (`ok/events`, `ok/folders/envelopes`, `ok/items` + `error`).
fn render_cycle(result: &CycleResult) -> Output {
    let sources = DaemonSources::from_cycle(&result.timeline, &result.mail, &result.rss);
    if crate::util::json_mode::is_json() {
        Output::Json(serde_json::json!({
            "ok": result.ok(),
            "started_at": result.started_at.to_rfc3339(),
            "timeline": sources.timeline,
            "mail": sources.mail,
            "rss": sources.rss,
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
        // Summary line first (aligned with `timeline sync`'s leading
        // summary); skipped actions don't count toward the total.
        let actions: Vec<&crate::modules::daemon::cycle::ActionResult> =
            [&result.timeline, &result.mail, &result.rss]
                .into_iter()
                .flatten()
                .collect();
        let executed = actions.len();
        let ok_count = actions.iter().filter(|a| a.ok).count();
        let mut text = if ok_count == executed {
            format!("synced {ok_count}/{executed} actions\n")
        } else {
            format!(
                "synced {ok_count}/{executed} actions ({} failed)\n",
                executed - ok_count
            )
        };
        for line in lines {
            text.push_str(&format!("{line}\n"));
        }
        Output::text(text)
    }
}

/// Write the final exit state (t4): on failure emit an `_error` record —
/// surfaced to daemon.log and stderr (WARN default) — and propagate so the
/// process exits 1 (the sync itself is not blocked; only the exit
/// finalization reports failure).
fn finalize_state(state: &DaemonState) -> Result<()> {
    if let Err(e) = state::write_result(state) {
        tracing::error!(
            target: "everyday",
            _error = "daemon_state_write_failed",
            message = %format!("daemon: final state write failed: {}", e.message()),
        );
        return Err(e);
    }
    Ok(())
}

/// Graceful shutdown (ADR F016, t5): the single exit path for the resident
/// daemon — mark the state stopped (`running=false` + `exit_at`/`exit_ok`)
/// and persist it. All stop sources (`--once` completion handled separately;
/// SIGINT / SIGTERM / Ctrl+C via the cancel token) converge here. The file
/// log needs no explicit close: `Sink::File` opens per write. A failed final
/// write surfaces as `_error` + exit 1 (t4).
fn graceful_shutdown(state_guard: &Mutex<DaemonState>) -> Result<()> {
    let mut state = state_guard.lock().unwrap_or_else(|e| e.into_inner());
    state.mark_exit(true);
    finalize_state(&state)
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

    // ── render_cycle (t4 shape alignment) ──

    fn sample_cycle() -> CycleResult {
        use crate::modules::daemon::cycle::ActionResult;
        CycleResult {
            started_at: chrono::Utc::now(),
            timeline: Some(ActionResult::timeline_ok(7)),
            mail: Some(ActionResult::mail_ok(4, 15)),
            rss: Some(ActionResult::rss_ok(3)),
        }
    }

    #[test]
    fn render_cycle_json_has_top_level_ok_and_source_fields() {
        crate::util::json_mode::set_json_mode(true);
        let out = render_cycle(&sample_cycle());
        crate::util::json_mode::set_json_mode(false);
        let v = match out {
            Output::Json(v) => v,
            other => panic!("expected JSON output, got {other:?}"),
        };
        assert_eq!(v["ok"], true);
        assert!(v.get("started_at").is_some(), "missing started_at");
        // Per-source fields only — no cross-contamination.
        assert_eq!(v["timeline"]["events"], 7);
        assert!(
            v["timeline"].get("folders").is_none(),
            "timeline must not carry mail fields"
        );
        assert_eq!(v["mail"]["folders"], 4);
        assert_eq!(v["mail"]["envelopes"], 15);
        assert!(
            v["mail"].get("events").is_none(),
            "mail must not carry timeline fields"
        );
        assert_eq!(v["rss"]["items"], 3);
    }

    #[test]
    fn render_cycle_json_null_for_skipped_source() {
        crate::util::json_mode::set_json_mode(true);
        let mut cycle = sample_cycle();
        cycle.rss = None;
        let out = render_cycle(&cycle);
        crate::util::json_mode::set_json_mode(false);
        let v = match out {
            Output::Json(v) => v,
            other => panic!("expected JSON output, got {other:?}"),
        };
        assert_eq!(v["rss"], serde_json::Value::Null);
    }

    #[test]
    fn render_cycle_text_has_summary_and_action_lines() {
        let out = render_cycle(&sample_cycle());
        let text = match out {
            Output::Text(t) => t,
            other => panic!("expected text output, got {other:?}"),
        };
        assert!(text.contains("synced 3/3 actions"), "{text}");
        assert!(text.contains("timeline: 7 events"), "{text}");
        assert!(text.contains("mail: 4 folders, 15 envelopes"), "{text}");
        assert!(text.contains("rss: 3 items"), "{text}");
    }

    #[test]
    fn render_cycle_text_failed_action_notes_failure() {
        use crate::modules::daemon::cycle::ActionResult;
        let mut cycle = sample_cycle();
        cycle.mail = Some(ActionResult::failed("connection refused"));
        let out = render_cycle(&cycle);
        let text = match out {
            Output::Text(t) => t,
            other => panic!("expected text output, got {other:?}"),
        };
        assert!(text.contains("synced 2/3 actions (1 failed)"), "{text}");
        assert!(text.contains("mail: failed: connection refused"), "{text}");
    }
}
