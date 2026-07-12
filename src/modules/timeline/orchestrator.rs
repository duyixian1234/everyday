//! Sync orchestrator: coordinates the sync of all providers.
//!
//! Responsibilities:
//! 1. Read the watermark of each (source, account) from `sync_state`.
//! 2. Build the `TimeWindow` (first sync looks back 30 days; cal looks ahead 7 days
//!    [L002](../../../docs/adr/L002-calendar-window-refresh.md)).
//! 3. Group by source and run provider sync in parallel (multiple accounts of the
//!    same source run serially).
//! 4. Write events to the events table by SyncMode (Append / WindowRefresh)
//!    [L001](../../../docs/adr/L001-append-only-event-log.md).
//! 5. Advance the watermark (updated on success, unchanged on failure).
//! 6. Return the statistics.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use futures::future::join_all;
use serde_json::{Value, json};

use crate::config::Config;
use crate::error::Result;
use crate::modules::timeline::providers::{build_providers, filter_providers};
use crate::modules::timeline::store;
use crate::modules::timeline::{ProviderStatus, ProviderSyncResult, TimelineProvider};
use crate::output::Output;

/// Default lookback days for the first sync.
const DEFAULT_LOOKBACK_DAYS: i64 = 30;
/// Cal window look-ahead days.
const CAL_LOOKAHEAD_DAYS: i64 = 7;

/// Run a full sync.
///
/// - `config`: provides account configuration.
/// - `sources`: source filter (empty = all).
/// - `since`: overrides the lookback window start (None = use watermark or default lookback).
pub async fn run_sync(
    config: &Arc<Config>,
    sources: &[String],
    since: Option<DateTime<Utc>>,
) -> Result<SyncOutput> {
    let pool = store::open().await?;
    let mut providers = build_providers(config);
    if !sources.is_empty() {
        providers = filter_providers(providers, sources);
    }

    let now = Utc::now();

    // Group by source (providers of the same source go in one group; serial within a group, parallel across groups).
    let groups = group_by_source(providers);

    let group_results: Vec<Vec<ProviderSyncResult>> = join_all(
        groups
            .into_iter()
            .map(|group| sync_group(&pool, group, now, since)),
    )
    .await;

    // Flatten results.
    let mut results: Vec<ProviderSyncResult> = Vec::new();
    for group in group_results {
        results.extend(group);
    }

    let output = SyncOutput { results };
    Ok(output)
}

/// Group providers by source (keeping the same source in one group).
fn group_by_source(
    providers: Vec<Box<dyn TimelineProvider>>,
) -> Vec<Vec<Box<dyn TimelineProvider>>> {
    let mut groups: Vec<(String, Vec<Box<dyn TimelineProvider>>)> = Vec::new();
    for p in providers {
        let source = p.source().to_string();
        if let Some((_, group)) = groups.iter_mut().find(|(s, _)| s == &source) {
            group.push(p);
        } else {
            groups.push((source, vec![p]));
        }
    }
    groups.into_iter().map(|(_, g)| g).collect()
}

/// Sync a group of providers (same source, serial execution).
async fn sync_group(
    pool: &sqlx::SqlitePool,
    providers: Vec<Box<dyn TimelineProvider>>,
    now: DateTime<Utc>,
    since_override: Option<DateTime<Utc>>,
) -> Vec<ProviderSyncResult> {
    let mut results = Vec::with_capacity(providers.len());
    for provider in providers {
        let result = sync_one(pool, provider, now, since_override).await;
        results.push(result);
    }
    results
}

/// Sync a single provider.
async fn sync_one(
    pool: &sqlx::SqlitePool,
    provider: Box<dyn TimelineProvider>,
    now: DateTime<Utc>,
    since_override: Option<DateTime<Utc>>,
) -> ProviderSyncResult {
    let source = provider.source().to_string();
    let account = provider.account().map(|s| s.to_string());

    // Read the watermark.
    let watermark = store::get_watermark(pool, &source, account.as_deref())
        .await
        .ok()
        .flatten();

    // Build the window.
    let from = since_override
        .or(watermark.as_ref().and_then(|w| w.last_sync))
        .unwrap_or_else(|| now - Duration::days(DEFAULT_LOOKBACK_DAYS));

    // Cal looks ahead 7 days.
    let to = if source == "cal" {
        now + Duration::days(CAL_LOOKAHEAD_DAYS)
    } else {
        now
    };

    let window = crate::modules::timeline::TimeWindow::new(from, to);

    // Run provider sync.
    match provider.sync(&window).await {
        Ok((events, mode)) => {
            // Write to the events table: on failure mark ProviderStatus::Failed instead of
            // silently swallowing it as Ok with 0 events (the old `.unwrap_or(0)` let DB
            // errors masquerade as "success, no new events") [L009](../../../docs/adr/L009-best-effort-sync.md).
            let count = match store::insert_events(pool, &events, mode, from, to).await {
                Ok(n) => n,
                Err(e) => {
                    eprintln!("timeline: insert_events failed for {source}: {e}");
                    return ProviderSyncResult {
                        source,
                        account,
                        events_count: 0,
                        status: ProviderStatus::Failed(format!("db write: {e}")),
                    };
                }
            };

            // Advance the watermark: failure is only warned, not fatal (next sync re-runs the
            // window, and INSERT OR IGNORE naturally de-duplicates - no double insertion)
            // [L009](../../../docs/adr/L009-best-effort-sync.md).
            if let Err(e) = store::set_watermark(pool, &source, account.as_deref(), now, true).await
            {
                eprintln!("timeline: set_watermark failed for {source}: {e} (will re-sync window)");
            }

            ProviderSyncResult {
                source,
                account,
                events_count: count,
                status: ProviderStatus::Ok,
            }
        }
        Err(e) => {
            // Failure: leave the watermark unchanged and retry next time.
            eprintln!("timeline: sync failed for {source}: {e}");
            ProviderSyncResult {
                source,
                account,
                events_count: 0,
                status: ProviderStatus::Failed(e.to_string()),
            }
        }
    }
}

/// Sync output result.
pub struct SyncOutput {
    pub results: Vec<ProviderSyncResult>,
}

impl SyncOutput {
    /// Render into an Output (text or JSON).
    pub fn to_output(&self, json_mode: bool) -> Output {
        let total = self.results.len();
        let ok_count = self
            .results
            .iter()
            .filter(|r| matches!(r.status, ProviderStatus::Ok))
            .count();
        let failed_count = total - ok_count;
        let total_events: usize = self.results.iter().map(|r| r.events_count).sum();

        if json_mode {
            let details: Vec<Value> = self
                .results
                .iter()
                .map(|r| {
                    let (status_str, error) = match &r.status {
                        ProviderStatus::Ok => ("ok", Value::Null),
                        ProviderStatus::Failed(msg) => ("failed", Value::String(msg.clone())),
                    };
                    json!({
                        "source": r.source,
                        "account": r.account,
                        "events": r.events_count,
                        "status": status_str,
                        "error": error,
                    })
                })
                .collect();

            Output::Json(json!({
                "providers_total": total,
                "providers_ok": ok_count,
                "providers_failed": failed_count,
                "events_synced": total_events,
                "details": details,
            }))
        } else {
            let mut text = format!("synced {ok_count}/{total} providers, {total_events} events\n");
            if failed_count > 0 {
                text = format!(
                    "synced {ok_count}/{total} providers, {total_events} events ({failed_count} failed)\n"
                );
            }
            text.push('\n');
            for r in &self.results {
                let acct = r.account.as_deref().unwrap_or("");
                let label = if acct.is_empty() {
                    r.source.clone()
                } else {
                    format!("{}[{}]", r.source, acct)
                };
                match &r.status {
                    ProviderStatus::Ok => {
                        text.push_str(&format!("  {:<20} {:>3} events\n", label, r.events_count));
                    }
                    ProviderStatus::Failed(msg) => {
                        text.push_str(&format!("  {:<20}   FAILED  {}\n", label, msg));
                    }
                }
            }
            Output::text(text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::timeline::{SyncMode, TimelineEvent, TimelineProvider};
    use async_trait::async_trait;

    /// Mock provider used to test the grouping logic of `group_by_source`.
    struct MockProvider {
        source_id: &'static str,
        account_name: Option<&'static str>,
    }

    #[async_trait]
    impl TimelineProvider for MockProvider {
        fn source(&self) -> &'static str {
            self.source_id
        }
        fn account(&self) -> Option<&str> {
            self.account_name
        }
        async fn sync(
            &self,
            _w: &crate::modules::timeline::TimeWindow,
        ) -> Result<(Vec<TimelineEvent>, SyncMode)> {
            Ok((vec![], SyncMode::Append))
        }
    }

    #[test]
    fn group_by_source_separates_sources() {
        let providers: Vec<Box<dyn TimelineProvider>> = vec![
            Box::new(MockProvider {
                source_id: "todo",
                account_name: Some("personal"),
            }),
            Box::new(MockProvider {
                source_id: "todo",
                account_name: Some("work"),
            }),
            Box::new(MockProvider {
                source_id: "note",
                account_name: Some("personal"),
            }),
            Box::new(MockProvider {
                source_id: "rss",
                account_name: None,
            }),
        ];
        let groups = group_by_source(providers);

        // 4 providers should split into 3 groups (todo/note/rss).
        assert_eq!(groups.len(), 3, "expected 3 groups");

        // Provider counts per group: todo=2, note=1, rss=1.
        let sizes: Vec<usize> = groups.iter().map(|g| g.len()).collect();
        let mut sizes_sorted = sizes.clone();
        sizes_sorted.sort_unstable();
        assert_eq!(sizes_sorted, vec![1, 1, 2]);

        // Verify the todo group contains 2 accounts.
        let todo_group = groups
            .iter()
            .find(|g| g[0].source() == "todo")
            .expect("todo group must exist");
        assert_eq!(todo_group.len(), 2);
        let mut accounts: Vec<&str> = todo_group.iter().map(|p| p.account().unwrap()).collect();
        accounts.sort_unstable();
        assert_eq!(accounts, vec!["personal", "work"]);
    }

    #[test]
    fn sync_output_text_mode() {
        let output = SyncOutput {
            results: vec![ProviderSyncResult {
                source: "todo".into(),
                account: Some("personal".into()),
                events_count: 3,
                status: ProviderStatus::Ok,
            }],
        };
        let text = match output.to_output(false) {
            Output::Text(s) => s,
            _ => panic!("expected text output"),
        };
        assert!(text.contains("synced 1/1"));
        assert!(text.contains("todo[personal]"));
        assert!(text.contains("3 events"));
    }

    #[test]
    fn sync_output_json_mode() {
        let output = SyncOutput {
            results: vec![
                ProviderSyncResult {
                    source: "mail".into(),
                    account: Some("work".into()),
                    events_count: 5,
                    status: ProviderStatus::Ok,
                },
                ProviderSyncResult {
                    source: "rss".into(),
                    account: None,
                    events_count: 0,
                    status: ProviderStatus::Failed("timeout".into()),
                },
            ],
        };
        let json = match output.to_output(true) {
            Output::Json(v) => v,
            _ => panic!("expected json output"),
        };
        assert_eq!(json["providers_total"], 2);
        assert_eq!(json["providers_ok"], 1);
        assert_eq!(json["providers_failed"], 1);
        assert_eq!(json["events_synced"], 5);
    }
}
