//! Cron parsing and the daemon's independent task scheduling loop.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

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
    crate::config::validate_task_schedule(expression)?;
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

/// Execute every task due at `now`, at most once per task, then roll its state
/// directly to the first occurrence after `now` (no backlog).
pub async fn run_due_tasks(
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
    }
    Ok(pass)
}

/// Independent resident loop. It shares only the daemon cancellation token;
/// its cadence is intentionally unrelated to the sync cycle interval.
pub async fn run_loop(config: Arc<Config>, shutdown: CancellationToken) -> Result<()> {
    let store = TaskStore::open_default().await?;
    loop {
        run_due_tasks(&config, &store, Local::now()).await?;
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
        let store = TaskStore::open_path(&path).await.unwrap();
        let now = Local::now();
        store
            .set_next_due(
                "missing",
                (now - chrono::Duration::minutes(1)).with_timezone(&Utc),
            )
            .await
            .unwrap();
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

        let pass = run_due_tasks(&config, &store, now).await.unwrap();
        assert_eq!(
            pass,
            SchedulerPass {
                scheduled: 1,
                ran: 1,
                failed: 1,
            }
        );
        let history = store.history("missing", 10).await.unwrap();
        assert_eq!(history.len(), 1);
        assert!(history[0].capture_output, "scheduled runs always capture");
        assert!(store.next_due("missing").await.unwrap().unwrap() > now.with_timezone(&Utc));
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
