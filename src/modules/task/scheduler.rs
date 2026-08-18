//! Cron parsing and the daemon's independent task scheduling loop.

use std::str::FromStr;
use std::time::Duration;

#[cfg(test)]
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, Utc};
use croner::Cron;
use tokio_util::sync::CancellationToken;

use crate::config::Config;
use crate::error::{AgentError, Result};
use crate::modules::task::runner::{self, RelayMode};
use crate::modules::task::store::TaskStore;

/// Fixed scheduler poll cadence (ADR F017).
pub const SCHEDULER_INTERVAL: Duration = Duration::from_secs(30);

/// Outcome of one scheduler pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SchedulerPass {
    pub scheduled: usize,
    pub ran: usize,
    pub failed: usize,
}

fn parse_schedule(expression: &str) -> Result<Cron> {
    Cron::from_str(expression.trim())
        .map_err(|e| AgentError::InvalidArgument(format!("invalid task schedule: {e}")))
}

/// Find the first occurrence strictly after `from`.
pub fn next_after(expression: &str, from: DateTime<Local>) -> Result<DateTime<Local>> {
    parse_schedule(expression)?
        .find_next_occurrence(&from, false)
        .map_err(|e| AgentError::Other(format!("cannot calculate next task occurrence: {e}")))
}

/// Initialize state from the previous minute so a matching current-minute
/// window is due, while older downtime windows are never backfilled.
pub fn initial_next_due(expression: &str, now: DateTime<Local>) -> Result<DateTime<Local>> {
    next_after(expression, now - chrono::Duration::minutes(1))
}

/// Whether any configured task carries a non-empty cron schedule.
fn any_scheduled(config: &Config) -> bool {
    config.tasks.values().any(|task| {
        task.schedule
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
    })
}

/// Executes every task due at `now`, at most once per task, then rolls its
/// state directly to the first occurrence after `now` (no backlog). A state
/// error on one task (DB read/write, next-occurrence calculation) is logged
/// and skipped so a single bad entry cannot abort the whole pass.
async fn run_due_tasks(
    config: &Config,
    store: &TaskStore,
    now: DateTime<Local>,
) -> Result<SchedulerPass> {
    let mut names: Vec<&String> = config.tasks.keys().collect();
    names.sort();
    let mut pass = SchedulerPass::default();

    for name in names {
        let task = &config.tasks[name];
        let Some(expression) = task
            .schedule
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        pass.scheduled += 1;
        if let Err(error) = run_due_task(store, name, task, expression, now, &mut pass).await {
            pass.failed += 1;
            tracing::error!(
                target: "everyday",
                _error = "scheduled_task_state_failed",
                task = %name,
                message = %error.message(),
            );
        }
    }
    Ok(pass)
}

/// Where the store is opened from, resolved lazily on the first `run_pass`.
enum StoreSource {
    /// The fixed `~/.config/everyday/task.db` (resolved at open time).
    Default,
    /// An explicit path (tests).
    #[cfg(test)]
    Path(PathBuf),
}

/// The daemon's task scheduler orchestration: a resident pass runner that owns
/// the store lifecycle and pass logging, so the resident loop and `--once`
/// share one entry point (architecture review, Candidate 1).
pub struct Scheduler {
    store: Option<TaskStore>,
    source: StoreSource,
}

impl Scheduler {
    /// Open the fixed `~/.config/everyday/task.db` on demand.
    pub fn new() -> Self {
        Self {
            store: None,
            source: StoreSource::Default,
        }
    }

    /// Open an explicit store path on demand (used by tests).
    #[cfg(test)]
    pub fn with_store_path(path: &Path) -> Self {
        Self {
            store: None,
            source: StoreSource::Path(path.to_path_buf()),
        }
    }

    /// Run one scheduler pass against `config`: gates on whether anything is
    /// scheduled, lazily opens the store, runs due tasks, and logs the pass
    /// summary. A store-open failure is an `Err` (the caller decides whether
    /// to propagate or continue).
    pub async fn run_pass(&mut self, config: &Config) -> Result<SchedulerPass> {
        if !any_scheduled(config) {
            return Ok(SchedulerPass::default());
        }
        if self.store.is_none() {
            let path = match &self.source {
                StoreSource::Default => crate::modules::task::store::task_db_path()?,
                #[cfg(test)]
                StoreSource::Path(path) => path.clone(),
            };
            self.store = Some(TaskStore::open_path(&path).await?);
        }
        let store = self.store.as_ref().expect("store opened above");
        let pass = run_due_tasks(config, store, Local::now()).await?;
        log_task_pass(pass);
        Ok(pass)
    }
}

/// Emit one `tracing::info!` pass summary (shared by resident and `--once`).
fn log_task_pass(pass: SchedulerPass) {
    tracing::info!(
        target: "everyday",
        _log = "task_scheduler_pass",
        scheduled = pass.scheduled,
        ran = pass.ran,
        failed = pass.failed,
    );
}

/// One task's due check: run at most once if `next_due <= now`, then roll the
/// persisted state forward to `cron.after(now)`.
async fn run_due_task(
    store: &TaskStore,
    name: &str,
    task: &crate::config::TaskConfig,
    expression: &str,
    now: DateTime<Local>,
    pass: &mut SchedulerPass,
) -> Result<()> {
    let persisted = store.next_due(name).await?;
    let due = match persisted {
        Some(due) => due,
        None => initial_next_due(expression, now)?.with_timezone(&Utc),
    };

    if due <= now.with_timezone(&Utc) {
        pass.ran += 1;
        match runner::run(store, name, task, &[], true, RelayMode::Silent).await {
            Ok(record) => {
                let ok = record.status == "success";
                if !ok {
                    pass.failed += 1;
                }
                tracing::info!(
                    target: "everyday",
                    _log = "scheduled_task_completed",
                    task = %name,
                    status = %record.status,
                    exit_code = record.exit_code,
                    duration_ms = record.duration_ms,
                );
            }
            Err(error) => {
                pass.failed += 1;
                tracing::error!(
                    target: "everyday",
                    _error = "scheduled_task_failed",
                    task = %name,
                    message = %error.message(),
                );
            }
        }
        let next = next_after(expression, now)?.with_timezone(&Utc);
        store.set_next_due(name, next).await?;
    } else if persisted.is_none() {
        store.set_next_due(name, due).await?;
    }
    Ok(())
}

/// Independent resident loop. It shares only the daemon cancellation token;
/// its cadence is intentionally unrelated to the sync cycle interval. The
/// config file is re-read every pass so task edits take effect without a
/// daemon restart, and the task database is opened only once a schedule
/// actually exists.
pub async fn run_loop(shutdown: CancellationToken) -> Result<()> {
    let mut config = crate::config::Config::load_or_default()?;
    let mut scheduler = Scheduler::new();
    loop {
        if let Err(error) = scheduler.run_pass(&config).await {
            tracing::error!(
                target: "everyday",
                _error = "task_scheduler_pass_failed",
                message = %error.message(),
            );
        }
        // Re-read config for the next pass; a malformed file (mid-edit) keeps
        // the last good config and is reported.
        match crate::config::Config::load_or_default() {
            Ok(next) => config = next,
            Err(error) => tracing::error!(
                target: "everyday",
                _error = "task_config_reload_failed",
                message = %error.message(),
            ),
        }
        tokio::select! {
            _ = tokio::time::sleep(SCHEDULER_INTERVAL) => {}
            _ = shutdown.cancelled() => break,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Timelike};
    use std::path::PathBuf;

    #[test]
    fn rejects_non_five_field_cron() {
        assert!(crate::config::validate_task_schedule("* * * * *").is_ok());
        assert!(crate::config::validate_task_schedule("0 * * * * *").is_err());
        assert!(crate::config::validate_task_schedule("* * * *").is_err());
    }

    #[test]
    fn initial_due_matches_current_minute_without_backlog() {
        let now = Local
            .with_ymd_and_hms(2026, 8, 18, 10, 16, 30)
            .single()
            .unwrap();
        let due = initial_next_due("* * * * *", now).unwrap();
        assert_eq!(due.minute(), 16);
        assert!(due <= now);
    }

    #[test]
    fn rolling_after_now_skips_missed_windows() {
        let now = Local
            .with_ymd_and_hms(2026, 8, 18, 10, 16, 30)
            .single()
            .unwrap();
        let next = next_after("* * * * *", now).unwrap();
        assert_eq!(next.minute(), 17);
        assert!(next > now);
    }

    #[tokio::test]
    async fn due_task_runs_once_records_and_rolls_forward() {
        let path: PathBuf = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("task-scheduler-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let now = Local::now();
        // Seed the next-due state so the due check fires on the first pass.
        {
            let store = TaskStore::open_path(&path).await.unwrap();
            store
                .set_next_due(
                    "missing",
                    (now - chrono::Duration::minutes(1)).with_timezone(&Utc),
                )
                .await
                .unwrap();
        }
        let mut config = Config::default();
        config.tasks.insert(
            "missing".into(),
            crate::config::TaskConfig {
                command: "everyday-command-that-does-not-exist".into(),
                args: String::new(),
                allow_extra_args: false,
                timeout_secs: 1,
                capture_output: false,
                schedule: Some("* * * * *".into()),
            },
        );

        let mut scheduler = Scheduler::with_store_path(&path);
        let pass = scheduler.run_pass(&config).await.unwrap();
        assert_eq!(
            pass,
            SchedulerPass {
                scheduled: 1,
                ran: 1,
                failed: 1,
            }
        );
        let store = TaskStore::open_path(&path).await.unwrap();
        let history = store.history("missing", 10).await.unwrap();
        assert_eq!(history.len(), 1);
        assert!(history[0].capture_output, "scheduled runs always capture");
        assert!(store.next_due("missing").await.unwrap().unwrap() > now.with_timezone(&Utc));
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn no_schedule_opens_no_store_and_returns_empty_pass() {
        let path: PathBuf = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("task-scheduler-empty-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        // No `[tasks.*]` with a schedule: the store must never be opened.
        let config = Config::default();
        let mut scheduler = Scheduler::with_store_path(&path);
        let pass = scheduler.run_pass(&config).await.unwrap();
        assert_eq!(pass, SchedulerPass::default());
        assert!(
            !path.exists(),
            "store must not be created when nothing is scheduled"
        );
        let _ = std::fs::remove_file(path);
    }
}
