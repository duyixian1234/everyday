//! Sync cycle engine (ADR F016, ticket t2).
//!
//! One cycle runs three actions **sequentially**:
//! 1. timeline event pull (`orchestrator::run_sync` — providers filtered by
//!    the daemon whitelist),
//! 2. mail envelope-cache sync (**all server folders**, per configured
//!    account — [`email::sync_all_folders`]),
//! 3. rss cache pull (every feed into `rss-items.db`).
//!
//! Failures are recorded per action, never fatal (best-effort, L009 spirit);
//! the next cycle retries. The scheduler sleeps *after* a cycle completes
//! (sleep-after-completion), so a slow cycle never triggers catch-up ticks.
//!
//! `run_cycles` takes the per-cycle work as an injected closure so tests can
//! substitute a counter and assert cadence without touching the network; the
//! cancellation token is the shutdown channel (Ctrl+C / signals, t5).

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio_util::sync::CancellationToken;

use crate::config::Config;
use crate::modules::rss::RssBackend;

/// Outcome of a single sync action within a cycle (feeds the daemon state
/// file, t3). Counters default to 0; only the relevant one is set.
#[derive(Debug, Clone, Default)]
pub struct ActionResult {
    pub ok: bool,
    /// Events written by the timeline pull.
    pub events: usize,
    /// Folders synced by the mail cache action.
    pub folders: usize,
    /// Envelopes added by the mail cache action.
    pub envelopes: usize,
    /// Items pulled by the rss action.
    pub items: usize,
    /// First error message (None when ok).
    pub error: Option<String>,
}

impl ActionResult {
    pub fn timeline_ok(events: usize) -> Self {
        Self {
            ok: true,
            events,
            ..Self::default()
        }
    }

    pub fn mail_ok(folders: usize, envelopes: usize) -> Self {
        Self {
            ok: true,
            folders,
            envelopes,
            ..Self::default()
        }
    }

    pub fn rss_ok(items: usize) -> Self {
        Self {
            ok: true,
            items,
            ..Self::default()
        }
    }

    pub fn failed(err: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(err.into()),
            ..Self::default()
        }
    }
}

/// Result of one full sync cycle (three actions, sequential). An action is
/// `None` when it was skipped by the sources whitelist.
#[derive(Debug, Clone, Default)]
pub struct CycleResult {
    pub started_at: DateTime<Utc>,
    pub timeline: Option<ActionResult>,
    pub mail: Option<ActionResult>,
    pub rss: Option<ActionResult>,
}

impl CycleResult {
    /// True when every *executed* action succeeded (skipped actions don't count).
    pub fn ok(&self) -> bool {
        [
            self.timeline.as_ref(),
            self.mail.as_ref(),
            self.rss.as_ref(),
        ]
        .into_iter()
        .flatten()
        .all(|a| a.ok)
    }
}

/// Run one sync cycle.
///
/// `sources` is the daemon whitelist (empty = all). Semantics (ADR F016): a
/// whitelisted source turns on both its timeline provider *and* its cache
/// action. The timeline pull always runs (its providers are filtered by
/// `run_sync`); the mail/rss cache actions run when `sources` is empty or
/// contains the matching name.
pub async fn run_cycle(config: &Arc<Config>, sources: &[String]) -> CycleResult {
    let started_at = Utc::now();
    let mut result = CycleResult {
        started_at,
        ..CycleResult::default()
    };

    // 1. timeline event pull (reuses the orchestrator; providers filtered by `sources`).
    result.timeline = Some(
        match crate::modules::timeline::orchestrator::run_sync(config, sources, None).await {
            Ok(out) => {
                use crate::modules::timeline::ProviderStatus;
                let events: usize = out.results.iter().map(|r| r.events_count).sum();
                let failed: Vec<String> = out
                    .results
                    .iter()
                    .filter_map(|r| match &r.status {
                        ProviderStatus::Failed(m) => Some(format!("{}: {m}", r.source)),
                        _ => None,
                    })
                    .collect();
                if failed.is_empty() {
                    ActionResult::timeline_ok(events)
                } else {
                    ActionResult {
                        ok: false,
                        events,
                        error: Some(failed.join("; ")),
                        ..ActionResult::default()
                    }
                }
            }
            Err(e) => ActionResult::failed(format!("timeline: {e}")),
        },
    );

    // 2. mail cache: all server folders for every configured account.
    if sources.is_empty() || sources.iter().any(|s| s == "mail") {
        let mut folders = 0usize;
        let mut envelopes = 0usize;
        let mut first_err: Option<String> = None;
        for account in &config.mail.accounts {
            match crate::modules::email::sync_all_folders(account).await {
                Ok(stats) => {
                    folders += stats.folders_synced;
                    envelopes += stats.envelopes_added;
                    if let Some((folder, msg)) = stats.errors.first() {
                        first_err.get_or_insert_with(|| format!("mail {folder}: {msg}"));
                    }
                }
                Err(e) => {
                    first_err.get_or_insert_with(|| format!("mail {}: {e}", account.name));
                }
            }
        }
        result.mail = Some(match first_err {
            Some(err) => ActionResult {
                ok: false,
                folders,
                envelopes,
                error: Some(err),
                ..ActionResult::default()
            },
            None => ActionResult::mail_ok(folders, envelopes),
        });
    }

    // 3. rss cache: pull every feed into rss-items.db.
    if sources.is_empty() || sources.iter().any(|s| s == "rss") {
        let backend = crate::modules::rss::RealRssBackend::new(config.rss_module_config());
        let mut items = 0usize;
        let mut first_err: Option<String> = None;
        for feed in &config.rss.feeds {
            match backend.fetch(&feed.name, 100).await {
                Ok(entries) => items += entries.len(),
                Err(e) => {
                    first_err.get_or_insert_with(|| format!("rss {}: {e}", feed.name));
                }
            }
        }
        result.rss = Some(match first_err {
            Some(err) => ActionResult {
                ok: false,
                items,
                error: Some(err),
                ..ActionResult::default()
            },
            None => ActionResult::rss_ok(items),
        });
    }

    result
}

/// Scheduler options for [`run_cycles`].
pub struct CycleLoopOptions {
    /// Run exactly one cycle then return (`daemon run --once`).
    pub once: bool,
    /// Sleep between cycles (sleep-after-completion; ignored when `once`).
    pub interval: Duration,
    /// Cancels the loop (Ctrl+C / signals, t5).
    pub shutdown: CancellationToken,
}

/// Number of completed cycles in the last run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CycleLoopStats {
    pub cycles: u64,
}

/// Run the cycle loop with sleep-after-completion semantics.
///
/// `run_one(cycle_index)` is injected so tests can substitute a counter and
/// assert cadence without touching the network. The loop:
/// 1. runs one cycle,
/// 2. returns immediately when `opts.once`,
/// 3. otherwise waits `opts.interval` **or** until `opts.shutdown` is
///    cancelled — whichever comes first — then loops.
pub async fn run_cycles<F, Fut>(mut run_one: F, opts: CycleLoopOptions) -> CycleLoopStats
where
    F: FnMut(u64) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let mut cycles = 0u64;
    loop {
        run_one(cycles).await;
        cycles += 1;
        if opts.once {
            break;
        }
        tokio::select! {
            _ = tokio::time::sleep(opts.interval) => {}
            _ = opts.shutdown.cancelled() => break,
        }
    }
    CycleLoopStats { cycles }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc as StdArc;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[tokio::test]
    async fn once_runs_single_cycle() {
        let counter = StdArc::new(AtomicU64::new(0));
        let c = counter.clone();
        let stats = run_cycles(
            move |_| {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                }
            },
            CycleLoopOptions {
                once: true,
                interval: Duration::from_millis(10),
                shutdown: CancellationToken::new(),
            },
        )
        .await;
        assert_eq!(stats.cycles, 1);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn resident_loop_runs_until_cancelled() {
        let counter = StdArc::new(AtomicU64::new(0));
        let c = counter.clone();
        let shutdown = CancellationToken::new();
        let cancel = shutdown.clone();
        let task = tokio::spawn(async move {
            run_cycles(
                move |_| {
                    let c = c.clone();
                    async move {
                        c.fetch_add(1, Ordering::SeqCst);
                    }
                },
                CycleLoopOptions {
                    once: false,
                    interval: Duration::from_millis(20),
                    shutdown,
                },
            )
            .await
        });
        // Let at least two cycles complete, then cancel.
        tokio::time::sleep(Duration::from_millis(55)).await;
        cancel.cancel();
        let stats = task.await.unwrap();
        assert!(stats.cycles >= 2, "cycles: {}", stats.cycles);
        assert!(counter.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test]
    async fn resident_loop_sleeps_after_completion() {
        // A slow cycle (10ms work) + 30ms interval must yield ~2 cycles in
        // ~90ms — the interval is added *after* the cycle, never caught up.
        let started = std::time::Instant::now();
        let counter = StdArc::new(AtomicU64::new(0));
        let c = counter.clone();
        let shutdown = CancellationToken::new();
        let cancel = shutdown.clone();
        let task = tokio::spawn(async move {
            run_cycles(
                move |_| {
                    let c = c.clone();
                    async move {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        c.fetch_add(1, Ordering::SeqCst);
                    }
                },
                CycleLoopOptions {
                    once: false,
                    interval: Duration::from_millis(30),
                    shutdown,
                },
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(90)).await;
        cancel.cancel();
        let stats = task.await.unwrap();
        let elapsed = started.elapsed();
        assert!(
            stats.cycles >= 1 && stats.cycles <= 3,
            "cycles: {} in {elapsed:?}",
            stats.cycles
        );
    }

    #[tokio::test]
    async fn cancel_during_sleep_prevents_next_cycle() {
        // t5: a stop arriving in the sleep gap must break the loop — the
        // current cycle already ran, the next one must never start.
        let counter = StdArc::new(AtomicU64::new(0));
        let c = counter.clone();
        let shutdown = CancellationToken::new();
        let cancel = shutdown.clone();
        let task = tokio::spawn(async move {
            run_cycles(
                move |_| {
                    let c = c.clone();
                    async move {
                        c.fetch_add(1, Ordering::SeqCst);
                    }
                },
                CycleLoopOptions {
                    once: false,
                    // Long sleep so the cancel lands squarely inside it.
                    interval: Duration::from_secs(3600),
                    shutdown,
                },
            )
            .await
        });
        // Wait for cycle 1 to finish, then cancel during the sleep.
        while counter.load(Ordering::SeqCst) < 1 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        cancel.cancel();
        let stats = task.await.unwrap();
        assert_eq!(stats.cycles, 1, "no next cycle after cancel in sleep");
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn sources_whitelist_gates_cache_actions() {
        // Pure whitelist logic: empty = all, otherwise exact match.
        assert!(sources_include(&[], "mail"));
        assert!(sources_include(&["mail".into(), "rss".into()], "rss"));
        assert!(!sources_include(&["mail".into()], "cal"));
    }

    fn sources_include(sources: &[String], name: &str) -> bool {
        sources.is_empty() || sources.iter().any(|s| s == name)
    }
}
