//! RSS/Atom subscription module.
//!
//! Full implementation: feed management (follow/list/unfollow, written to `[[rss.feeds]]`)
//! plus fetch aggregation (digest/fetch, reqwest fetch + feed-rs parse).
//!
//! Design notes [F008](../../docs/adr/F008-rss-module.md):
//! - Feeds are public URLs, so **no keyring is needed** (unlike mail/cal).
//! - Config writes use a localized toml::Value edit (touching only the `rss.feeds` array),
//!   preserving mail/calendar and other sections and their field order to avoid clobbering
//!   other accounts.
//! - digest/fetch fetch concurrently with reqwest (with timeout + UA) and parse with feed-rs;
//!   a single feed failure is non-fatal (best-effort), consistent with the calendar module's
//!   per-calendar failure degradation.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::future::join_all;

use crate::config::{Config, RssFeed};
use crate::error::{AgentError, Result};
use crate::modules::{Executor, parse_simple_args};
use crate::output::Output;

/// Display row for a single aggregated entry, plus its sort key.
struct EntryRow {
    feed: String,
    title: String,
    published: String,
    author: String,
    link: String,
    /// Publish time used for sorting (entries without a time sort last).
    sort_key: Option<DateTime<Utc>>,
}

/// Result of one fetch: `feed` on success, `error` on failure.
struct FetchedFeed {
    name: String,
    feed: Option<feed_rs::model::Feed>,
    error: Option<String>,
}

pub struct RssModule {
    config: Arc<Config>,
}

impl RssModule {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Executor for RssModule {
    fn description(&self) -> &'static str {
        "RSS/Atom feed reader: follow, list, unfollow, aggregate (digest), fetch."
    }

    fn module_arg_spec(&self) -> crate::modules::ModuleArgSpec {
        use crate::modules::{ActionArgSpec, ArgKind, ArgSpec, ModuleArgSpec, Positional};
        static ACTIONS: &[ActionArgSpec] = &[
            ActionArgSpec {
                name: "follow",
                description: "关注一个 RSS/Atom feed",
                usage: "everyday rss follow --name N --url URL [--category C]",
                args: &[
                    ArgSpec {
                        name: "name",
                        help: "feed 名称",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "url",
                        help: "feed URL（http/https）",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "category",
                        help: "分类",
                        kind: ArgKind::Value,
                    },
                ],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "list",
                description: "列出已关注的 feed",
                usage: "everyday rss list",
                args: &[],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "unfollow",
                description: "取消关注",
                usage: "everyday rss unfollow --name N",
                args: &[ArgSpec {
                    name: "name",
                    help: "feed 名称",
                    kind: ArgKind::Value,
                }],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "digest",
                description: "生成今日早报摘要",
                usage: "everyday rss digest [--limit N] [--name FEED] [--category C]",
                args: &[
                    ArgSpec {
                        name: "limit",
                        help: "条数上限",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "name",
                        help: "按 feed 名过滤",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "category",
                        help: "按分类过滤",
                        kind: ArgKind::Value,
                    },
                ],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "fetch",
                description: "抓取并展示某个 feed 的文章",
                usage: "everyday rss fetch --name N [--limit N]",
                args: &[
                    ArgSpec {
                        name: "name",
                        help: "feed 名称",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "limit",
                        help: "条数上限",
                        kind: ArgKind::Value,
                    },
                ],
                positional: Positional::None,
            },
        ];
        ModuleArgSpec {
            name: "rss",
            description: self.description(),
            actions: ACTIONS,
        }
    }

    async fn execute(&self, action: &str, args: &[String]) -> Result<Output> {
        let (flags, _positional) = parse_simple_args(args);
        match action {
            "follow" => rss_follow(&flags).await,
            "list" => rss_list(&self.config),
            "unfollow" => rss_unfollow(&flags).await,
            "digest" => rss_digest(&self.config, &flags).await,
            "fetch" => rss_fetch(&self.config, &flags).await,
            other => Err(AgentError::UnknownAction(format!("rss {other}"))),
        }
    }
}

// ============ Config read/write (localized edit of rss.feeds) ============

/// Read the config file into a toml::Value (empty table if absent/empty).
fn load_config_value() -> Result<toml::Value> {
    let path = Config::config_path()?;
    if !path.exists() {
        return Ok(toml::Value::Table(toml::value::Table::new()));
    }
    let text = std::fs::read_to_string(&path)?;
    if text.trim().is_empty() {
        return Ok(toml::Value::Table(toml::value::Table::new()));
    }
    Ok(toml::from_str(&text)?)
}

/// Write a toml::Value back to the config file (creating the parent dir).
fn save_config_value(root: &toml::Value) -> Result<()> {
    let path = Config::config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(root)
        .map_err(|e| AgentError::Config(format!("serialize config: {e}")))?;
    std::fs::write(&path, text)?;
    Ok(())
}

/// Append a feed to `rss.feeds` (creating the table/array if absent).
/// Error if a feed with the same name already exists, to avoid duplicates.
fn append_feed(root: &mut toml::Value, feed: &RssFeed) -> Result<()> {
    let table = root
        .as_table_mut()
        .ok_or_else(|| AgentError::Config("config root is not a table".into()))?;
    let rss = table
        .entry("rss")
        .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
    let rss_table = rss
        .as_table_mut()
        .ok_or_else(|| AgentError::Config("rss is not a table".into()))?;
    let feeds = rss_table
        .entry("feeds")
        .or_insert_with(|| toml::Value::Array(Vec::new()));
    let arr = feeds
        .as_array_mut()
        .ok_or_else(|| AgentError::Config("rss.feeds is not an array".into()))?;

    if arr
        .iter()
        .any(|f| f.get("name").and_then(|n| n.as_str()) == Some(&feed.name))
    {
        return Err(AgentError::InvalidArgument(format!(
            "feed '{}' already exists",
            feed.name
        )));
    }

    let mut entry = toml::value::Table::new();
    entry.insert("name".into(), toml::Value::String(feed.name.clone()));
    entry.insert("url".into(), toml::Value::String(feed.url.clone()));
    if let Some(cat) = &feed.category {
        entry.insert("category".into(), toml::Value::String(cat.clone()));
    }
    arr.push(toml::Value::Table(entry));
    Ok(())
}

/// Remove the feed with the given name from `rss.feeds`; returns whether anything was removed.
fn remove_feed(root: &mut toml::Value, name: &str) -> Result<bool> {
    let Some(rss) = root.as_table_mut().and_then(|t| t.get_mut("rss")) else {
        return Ok(false);
    };
    let Some(feeds) = rss.as_table_mut().and_then(|t| t.get_mut("feeds")) else {
        return Ok(false);
    };
    let Some(arr) = feeds.as_array_mut() else {
        return Ok(false);
    };
    let before = arr.len();
    arr.retain(|f| f.get("name").and_then(|n| n.as_str()) != Some(name));
    Ok(arr.len() < before)
}

/// Filter feeds: `--name` and `--category` match exactly (case-sensitive).
///
/// If `--name` is given but no feed matches, return `InvalidArgument` (not found).
fn filter_feeds(feeds: &[RssFeed], flags: &HashMap<String, String>) -> Result<Vec<RssFeed>> {
    let name = flags.get("name");
    let category = flags.get("category");
    let mut out = Vec::new();
    for f in feeds {
        if let Some(n) = name
            && &f.name != n
        {
            continue;
        }
        if let Some(c) = category {
            match &f.category {
                Some(fc) if fc == c => {}
                _ => continue,
            }
        }
        out.push(f.clone());
    }
    if let Some(n) = name
        && out.is_empty()
    {
        return Err(AgentError::InvalidArgument(format!("feed '{n}' not found")));
    }
    Ok(out)
}

// ============ Action implementations ============

/// `rss follow --name N --url URL [--category C]`: write to the config file.
async fn rss_follow(flags: &HashMap<String, String>) -> Result<Output> {
    let name = flags
        .get("name")
        .ok_or_else(|| AgentError::InvalidArgument("follow requires --name <name>".into()))?;
    let url = flags
        .get("url")
        .ok_or_else(|| AgentError::InvalidArgument("follow requires --url <url>".into()))?;
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(AgentError::InvalidArgument(format!(
            "invalid feed url (must start with http:// or https://): {url}"
        )));
    }
    let category = flags.get("category").cloned();
    let feed = RssFeed {
        name: name.clone(),
        url: url.clone(),
        category,
    };

    let mut root = load_config_value()?;
    append_feed(&mut root, &feed)?;
    save_config_value(&root)?;
    Ok(Output::text(format!(
        "followed feed '{}' ({})",
        feed.name, feed.url
    )))
}

/// `rss list`: list all subscribed feeds (headers: name / URL / category).
fn rss_list(config: &Config) -> Result<Output> {
    let rows = config
        .rss
        .feeds
        .iter()
        .map(|f| {
            vec![
                f.name.clone(),
                f.url.clone(),
                f.category.clone().unwrap_or_default(),
            ]
        })
        .collect();
    Ok(Output::records(
        vec!["name".into(), "url".into(), "category".into()],
        rows,
    ))
}

/// `rss unfollow --name N`: remove from the config file.
async fn rss_unfollow(flags: &HashMap<String, String>) -> Result<Output> {
    let name = flags
        .get("name")
        .ok_or_else(|| AgentError::InvalidArgument("unfollow requires --name <name>".into()))?;
    let mut root = load_config_value()?;
    let removed = remove_feed(&mut root, name)?;
    if !removed {
        return Err(AgentError::InvalidArgument(format!(
            "feed '{name}' not found"
        )));
    }
    save_config_value(&root)?;
    Ok(Output::text(format!("unfollowed feed '{name}'")))
}

/// `rss digest [--limit N] [--name FEED] [--category C]`: concurrent fetch, aggregate, sort by time descending.
async fn rss_digest(config: &Config, flags: &HashMap<String, String>) -> Result<Output> {
    let feeds = filter_feeds(&config.rss.feeds, flags)?;
    if feeds.is_empty() {
        return Err(AgentError::InvalidArgument(
            "no feeds to fetch (add one with `everyday rss follow --name N --url URL`)".into(),
        ));
    }

    let client = build_client()?;
    let tasks: Vec<_> = feeds.iter().map(|f| fetch_one(&client, f)).collect();
    let results = join_all(tasks).await;

    let mut rows: Vec<EntryRow> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    for r in results {
        match r.feed {
            Some(f) => {
                for e in &f.entries {
                    rows.push(build_entry_row(&r.name, e));
                }
            }
            None => errors.push(format!("{}: {}", r.name, r.error.unwrap_or_default())),
        }
    }

    // All failed -> error; partially failed -> still output the successful part (best-effort).
    if rows.is_empty() && !errors.is_empty() {
        return Err(AgentError::Network(errors.join("; ")));
    }

    rows.sort_by(|a, b| cmp_opt_dt_desc(&a.sort_key, &b.sort_key));
    let limit = flags
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(30);
    rows.truncate(limit);

    let out_rows = rows
        .into_iter()
        .map(|r| vec![r.feed, r.title, r.published, r.author, r.link])
        .collect();
    Ok(Output::records(
        vec![
            "feed".into(),
            "title".into(),
            "published".into(),
            "author".into(),
            "link".into(),
        ],
        out_rows,
    ))
}

/// `rss fetch --name N [--limit N]`: fetch a single feed and list its entries.
async fn rss_fetch(config: &Config, flags: &HashMap<String, String>) -> Result<Output> {
    let name = flags
        .get("name")
        .ok_or_else(|| AgentError::InvalidArgument("fetch requires --name <name>".into()))?;
    let feed = config
        .rss
        .feeds
        .iter()
        .find(|f| &f.name == name)
        .ok_or_else(|| AgentError::InvalidArgument(format!("feed '{name}' not found")))?;

    let client = build_client()?;
    let res = fetch_one(&client, feed).await;
    let f = res
        .feed
        .ok_or_else(|| AgentError::Network(res.error.unwrap_or_else(|| "fetch failed".into())))?;

    let mut rows: Vec<EntryRow> = f
        .entries
        .iter()
        .map(|e| build_entry_row(&feed.name, e))
        .collect();
    rows.sort_by(|a, b| cmp_opt_dt_desc(&a.sort_key, &b.sort_key));
    let limit = flags
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(20);
    rows.truncate(limit);

    let out_rows = rows
        .into_iter()
        .map(|r| vec![r.title, r.published, r.author, r.link])
        .collect();
    Ok(Output::records(
        vec![
            "title".into(),
            "published".into(),
            "author".into(),
            "link".into(),
        ],
        out_rows,
    ))
}

// ============ Network fetch ============

/// Build a reqwest client with timeout and UA (rustls-tls, reusing the ring provider installed by main.rs).
fn build_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(format!("everyday/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| AgentError::Network(format!("build http client: {e}")))
}

/// Fetch a single feed and parse it into a Feed (returns an error on failure, never panics).
async fn fetch_one(client: &reqwest::Client, feed: &RssFeed) -> FetchedFeed {
    match client.get(&feed.url).send().await {
        Ok(resp) => {
            if !resp.status().is_success() {
                return FetchedFeed {
                    name: feed.name.clone(),
                    feed: None,
                    error: Some(format!("HTTP {}", resp.status())),
                };
            }
            let bytes = match resp.bytes().await {
                Ok(b) => b,
                Err(e) => {
                    return FetchedFeed {
                        name: feed.name.clone(),
                        feed: None,
                        error: Some(format!("read body: {e}")),
                    };
                }
            };
            match feed_rs::parser::parse(bytes.as_ref()) {
                Ok(f) => FetchedFeed {
                    name: feed.name.clone(),
                    feed: Some(f),
                    error: None,
                },
                Err(e) => FetchedFeed {
                    name: feed.name.clone(),
                    feed: None,
                    error: Some(format!("parse: {e}")),
                },
            }
        }
        Err(e) => FetchedFeed {
            name: feed.name.clone(),
            feed: None,
            error: Some(e.to_string()),
        },
    }
}

// ============ Entry row construction ============

/// Build a display row from a feed-rs Entry.
fn build_entry_row(feed_name: &str, entry: &feed_rs::model::Entry) -> EntryRow {
    let title = entry
        .title
        .as_ref()
        .map(|t| t.content.clone())
        .unwrap_or_default();
    let published = entry.published;
    let author = entry
        .authors
        .first()
        .map(|p| p.name.clone())
        .unwrap_or_default();
    let link = pick_link(&entry.links);
    let published_str = published
        .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "—".into());
    EntryRow {
        feed: feed_name.to_string(),
        title,
        published: published_str,
        author,
        link,
        sort_key: published,
    }
}

/// Pick the display link: prefer `rel="alternate"`, else the first one; empty string if none.
fn pick_link(links: &[feed_rs::model::Link]) -> String {
    links
        .iter()
        .find(|l| l.rel.as_deref() == Some("alternate"))
        .or_else(|| links.first())
        .map(|l| l.href.clone())
        .unwrap_or_default()
}

/// Compare by publish time, descending (entries without a time sort last).
fn cmp_opt_dt_desc(a: &Option<DateTime<Utc>>, b: &Option<DateTime<Utc>>) -> std::cmp::Ordering {
    match (a, b) {
        (Some(x), Some(y)) => y.cmp(x), // descending: newest first
        (Some(_), None) => std::cmp::Ordering::Less, // dated entries sort before undated
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

// ============ Timeline data ingestion ============

/// Timeline ingestion: raw RSS entry data.
pub struct RssTimelineEntry {
    pub feed_name: String,
    pub feed_url: String,
    pub title: String,
    pub summary: String,
    pub link: String,
    pub author: String,
    pub published: Option<DateTime<Utc>>,
    pub guid: String,
}

/// Timeline incremental fetch: fetch all feeds, return entries whose publish time falls in the window.
pub async fn fetch_for_timeline(
    config: &Config,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<RssTimelineEntry>> {
    if config.rss.feeds.is_empty() {
        return Ok(Vec::new());
    }
    let client = build_client()?;
    let tasks: Vec<_> = config
        .rss
        .feeds
        .iter()
        .map(|f| fetch_one(&client, f))
        .collect();
    let results = join_all(tasks).await;

    let mut entries = Vec::new();
    for r in &results {
        if let Some(f) = &r.feed {
            for e in &f.entries {
                let published = e.published;
                // Filter: keep entries whose publish time is within the window (skip those without one).
                if let Some(pub_dt) = published
                    && (pub_dt < from || pub_dt > to)
                {
                    continue;
                }
                let title = e
                    .title
                    .as_ref()
                    .map(|t| t.content.clone())
                    .unwrap_or_default();
                let summary = e
                    .summary
                    .as_ref()
                    .map(|s| {
                        let content = s.content.as_str();
                        if content.len() > 200 {
                            format!("{}...", &content[..200])
                        } else {
                            content.to_string()
                        }
                    })
                    .unwrap_or_default();
                let author = e
                    .authors
                    .first()
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                let link = pick_link(&e.links);
                let guid = if !e.id.is_empty() {
                    e.id.clone()
                } else if !link.is_empty() {
                    link.clone()
                } else {
                    String::new()
                };
                let feed_url = config
                    .rss
                    .feeds
                    .iter()
                    .find(|f| f.name == r.name)
                    .map(|f| f.url.clone())
                    .unwrap_or_default();
                entries.push(RssTimelineEntry {
                    feed_name: r.name.clone(),
                    feed_url,
                    title,
                    summary,
                    link,
                    author,
                    published,
                    guid,
                });
            }
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// Minimal Atom sample used to parse into an Entry (author/link semantics are clearer than RSS2).
    const SAMPLE_RSS: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Test Feed</title>
  <entry>
    <title>Hello World</title>
    <link href="http://example.com/1"/>
    <published>2026-07-09T14:00:00Z</published>
    <updated>2026-07-09T14:00:00Z</updated>
    <author><name>Bob</name></author>
  </entry>
  <entry>
    <title>Second</title>
    <link rel="self" href="http://example.com/2"/>
  </entry>
</feed>"#;

    #[test]
    fn append_and_remove_feed_on_value() {
        let mut root = toml::Value::Table(toml::value::Table::new());
        // Seed a mail account to verify the localized edit does not corrupt it.
        // Note: toml::Value indexing does not auto-insert; insert must be used explicitly.
        root.as_table_mut().unwrap().insert(
            "mail".into(),
            toml::from_str("accounts = [{ name = 'work' }]").unwrap(),
        );

        let feed = RssFeed {
            name: "hn".into(),
            url: "https://hnrss.org/frontpage".into(),
            category: Some("tech".into()),
        };
        append_feed(&mut root, &feed).unwrap();
        // Adding a duplicate should error.
        assert!(append_feed(&mut root, &feed).is_err());

        // rss.feeds now contains one feed.
        let arr = root
            .get("rss")
            .and_then(|r| r.get("feeds"))
            .and_then(|f| f.as_array())
            .unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0].get("name").unwrap().as_str(), Some("hn"));
        assert_eq!(arr[0].get("category").unwrap().as_str(), Some("tech"));

        // The mail account was not corrupted.
        assert_eq!(
            root.get("mail")
                .and_then(|m| m.get("accounts"))
                .and_then(|a| a.as_array())
                .unwrap()
                .len(),
            1
        );

        // Remove.
        assert!(remove_feed(&mut root, "hn").unwrap());
        assert!(!remove_feed(&mut root, "hn").unwrap()); // a second remove returns false
    }

    #[test]
    fn filter_feeds_by_name_and_category() {
        let feeds = vec![
            RssFeed {
                name: "a".into(),
                url: "u1".into(),
                category: Some("tech".into()),
            },
            RssFeed {
                name: "b".into(),
                url: "u2".into(),
                category: None,
            },
            RssFeed {
                name: "c".into(),
                url: "u3".into(),
                category: Some("tech".into()),
            },
        ];
        // By category tech.
        let f = filter_feeds(&feeds, &HashMap::from([("category".into(), "tech".into())])).unwrap();
        assert_eq!(f.len(), 2);
        // Exact match by name.
        let f = filter_feeds(&feeds, &HashMap::from([("name".into(), "b".into())])).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "b");
        // Name not found -> error.
        assert!(filter_feeds(&feeds, &HashMap::from([("name".into(), "z".into())])).is_err());
    }

    #[test]
    fn cmp_opt_dt_desc_sorts_correctly() {
        let t1 = Utc.with_ymd_and_hms(2026, 7, 9, 14, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 7, 8, 14, 0, 0).unwrap();
        // The newer (t1) should sort before the undated.
        assert_eq!(cmp_opt_dt_desc(&Some(t1), &None), std::cmp::Ordering::Less);
        // Descending: t1 (new) should precede t2 (old) -> t1.cmp(t2) yields Less.
        assert_eq!(
            cmp_opt_dt_desc(&Some(t1), &Some(t2)),
            std::cmp::Ordering::Less
        );
        // Two undated entries are equal.
        assert_eq!(cmp_opt_dt_desc(&None, &None), std::cmp::Ordering::Equal);
    }

    #[test]
    fn pick_link_prefers_alternate() {
        use feed_rs::model::Link;
        let links = vec![
            Link {
                href: "http://x/self".into(),
                rel: Some("self".into()),
                media_type: None,
                href_lang: None,
                title: None,
                length: None,
            },
            Link {
                href: "http://x/alt".into(),
                rel: Some("alternate".into()),
                media_type: None,
                href_lang: None,
                title: None,
                length: None,
            },
        ];
        assert_eq!(pick_link(&links), "http://x/alt");
        // When no alternate, take the first.
        let links = vec![Link {
            href: "http://x/only".into(),
            rel: Some("self".into()),
            media_type: None,
            href_lang: None,
            title: None,
            length: None,
        }];
        assert_eq!(pick_link(&links), "http://x/only");
        assert_eq!(pick_link(&[]), "");
    }

    #[test]
    fn rss_list_renders_rows() {
        let cfg = Config {
            rss: crate::config::RssConfig {
                feeds: vec![
                    RssFeed {
                        name: "a".into(),
                        url: "u1".into(),
                        category: None,
                    },
                    RssFeed {
                        name: "b".into(),
                        url: "u2".into(),
                        category: Some("cat".into()),
                    },
                ],
            },
            ..Default::default()
        };
        let out = rss_list(&cfg).unwrap();
        if let Output::Records { headers, rows } = out {
            assert_eq!(headers, vec!["name", "url", "category"]);
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0][0], "a");
            assert_eq!(rows[1][2], "cat");
        } else {
            panic!("expected Records output");
        }
    }

    #[test]
    fn build_entry_row_from_parsed_rss() {
        let feed = feed_rs::parser::parse(SAMPLE_RSS.as_bytes()).expect("parse sample rss");
        assert_eq!(feed.entries.len(), 2);

        // First entry: has title/link/author/time.
        let row = build_entry_row("test", &feed.entries[0]);
        assert_eq!(row.title, "Hello World");
        assert_eq!(row.link, "http://example.com/1"); // Atom link defaults to rel=alternate
        assert_eq!(row.author, "Bob");
        assert!(row.published.contains("2026"));
        assert!(row.sort_key.is_some());

        // Second entry: no author/no time -> falls back to empty string and placeholder.
        let row = build_entry_row("test", &feed.entries[1]);
        assert_eq!(row.title, "Second");
        assert_eq!(row.link, "http://example.com/2"); // when no alternate, take the first link
        assert_eq!(row.author, "");
        assert!(row.sort_key.is_none());
        assert_eq!(row.published, "—");
    }
}
