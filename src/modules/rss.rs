//! RSS/Atom 订阅模块。
//!
//! 完整实现：订阅源管理（follow/list/unfollow，写入 `[[rss.feeds]]`）+ 抓取聚合
//! （digest/fetch，reqwest 抓取 + feed-rs 解析）。
//!
//! 设计要点：
//! - 订阅源是公开 URL，**不需要密钥环**（与 mail/cal 不同）。
//! - 配置文件写入采用 toml::Value 局部编辑（只动 `rss.feeds` 数组），
//!   保留 mail/calendar 等其他段落与字段顺序，避免误改他人账户。
//! - digest/fetch 用 reqwest（带超时 + UA）并发抓取，feed-rs 解析，
//!   单源失败不致命（best-effort），与 calendar 模块单日历失败降级一致。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::future::join_all;

use crate::config::{Config, RssFeed};
use crate::error::{AgentError, Result};
use crate::modules::{ActionDoc, Executor, parse_simple_args};
use crate::output::Output;

/// 单条聚合条目的展示行 + 排序键。
struct EntryRow {
    feed: String,
    title: String,
    published: String,
    author: String,
    link: String,
    /// 用于排序的发布时间（无时间排最后）。
    sort_key: Option<DateTime<Utc>>,
}

/// 一次抓取的结果：成功带 `feed`，失败带 `error`。
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
    fn name(&self) -> &'static str {
        "rss"
    }

    fn description(&self) -> &'static str {
        "RSS/Atom feed reader: follow, list, unfollow, aggregate (digest), fetch."
    }

    fn actions(&self) -> Vec<ActionDoc> {
        vec![
            ActionDoc::new(
                "follow",
                "Add a feed to config",
                "everyday rss follow --name N --url URL [--category C]",
            ),
            ActionDoc::new("list", "List followed feeds", "everyday rss list"),
            ActionDoc::new(
                "unfollow",
                "Remove a feed from config",
                "everyday rss unfollow --name N",
            ),
            ActionDoc::new(
                "digest",
                "Aggregate recent entries from all/selected feeds",
                "everyday rss digest [--limit N] [--name FEED] [--category C]",
            ),
            ActionDoc::new(
                "fetch",
                "Fetch a single feed and list its entries",
                "everyday rss fetch --name N [--limit N]",
            ),
        ]
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

// ============ 配置读写（局部编辑 rss.feeds） ============

/// 读取配置文件为 toml::Value（不存在/空则空表）。
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

/// 把 toml::Value 写回配置文件（自动建父目录）。
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

/// 向 `rss.feeds` 追加一个订阅源（创建表/数组若不存在）。
/// 同名源已存在则报错，避免重复。
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

/// 从 `rss.feeds` 删除指定名字的源，返回是否真的删除了。
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

/// 过滤订阅源：`--name` 精确匹配，`--category` 精确匹配（区分大小写）。
///
/// 若指定了 `--name` 却没有任何源匹配，返回 `InvalidArgument`（提示未找到）。
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

// ============ 动作实现 ============

/// `rss follow --name N --url URL [--category C]`：写入配置文件。
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

/// `rss list`：列出所有订阅源（表头：名称 / URL / 分类）。
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

/// `rss unfollow --name N`：从配置文件删除。
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

/// `rss digest [--limit N] [--name FEED] [--category C]`：并发抓取、聚合、按时间降序。
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

    // 全部失败 → 报错；部分失败 → 继续输出成功部分（best-effort）。
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

/// `rss fetch --name N [--limit N]`：抓取单个源并列出其条目。
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

// ============ 网络抓取 ============

/// 构建带超时与 UA 的 reqwest 客户端（rustls-tls，复用 main.rs 安装的 ring provider）。
fn build_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(format!("everyday/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| AgentError::Network(format!("build http client: {e}")))
}

/// 抓取单个源并解析为 feed（失败返回 error，不抛）。
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

// ============ 条目行构造 ============

/// 从 feed-rs 的 Entry 构造展示行。
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

/// 选展示链接：优先 `rel="alternate"`，否则取第一个；都无则空串。
fn pick_link(links: &[feed_rs::model::Link]) -> String {
    links
        .iter()
        .find(|l| l.rel.as_deref() == Some("alternate"))
        .or_else(|| links.first())
        .map(|l| l.href.clone())
        .unwrap_or_default()
}

/// 按发布时间降序比较（无时间排最后）。
fn cmp_opt_dt_desc(a: &Option<DateTime<Utc>>, b: &Option<DateTime<Utc>>) -> std::cmp::Ordering {
    match (a, b) {
        (Some(x), Some(y)) => y.cmp(x),              // 降序：新的在前
        (Some(_), None) => std::cmp::Ordering::Less, // 有时间的排在无时间之前
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// 最简 Atom 样例，供解析构造 Entry 用（author/link 语义比 RSS2 清晰）。
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
        // 预置一个 mail 账户，验证局部编辑不会破坏它。
        // 注意：toml::Value 索引不自动插入，必须用 insert。
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
        // 重复添加应报错。
        assert!(append_feed(&mut root, &feed).is_err());

        // rss.feeds 含一个源。
        let arr = root
            .get("rss")
            .and_then(|r| r.get("feeds"))
            .and_then(|f| f.as_array())
            .unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0].get("name").unwrap().as_str(), Some("hn"));
        assert_eq!(arr[0].get("category").unwrap().as_str(), Some("tech"));

        // mail 账户未被破坏。
        assert_eq!(
            root.get("mail")
                .and_then(|m| m.get("accounts"))
                .and_then(|a| a.as_array())
                .unwrap()
                .len(),
            1
        );

        // 删除。
        assert!(remove_feed(&mut root, "hn").unwrap());
        assert!(!remove_feed(&mut root, "hn").unwrap()); // 再删返回 false
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
        // 按分类 tech。
        let f = filter_feeds(&feeds, &HashMap::from([("category".into(), "tech".into())])).unwrap();
        assert_eq!(f.len(), 2);
        // 按名字精确匹配。
        let f = filter_feeds(&feeds, &HashMap::from([("name".into(), "b".into())])).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "b");
        // 名字不存在 → 报错。
        assert!(filter_feeds(&feeds, &HashMap::from([("name".into(), "z".into())])).is_err());
    }

    #[test]
    fn cmp_opt_dt_desc_sorts_correctly() {
        let t1 = Utc.with_ymd_and_hms(2026, 7, 9, 14, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 7, 8, 14, 0, 0).unwrap();
        // 新的（t1）应排在无时间之前。
        assert_eq!(cmp_opt_dt_desc(&Some(t1), &None), std::cmp::Ordering::Less);
        // 降序：t1(新) 应在 t2(旧) 之前 → t1.cmp(t2) 给 Less。
        assert_eq!(
            cmp_opt_dt_desc(&Some(t1), &Some(t2)),
            std::cmp::Ordering::Less
        );
        // 两个无时间的相等。
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
        // 无 alternate 时取第一个。
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

        // 第一条：有标题/链接/作者/时间。
        let row = build_entry_row("test", &feed.entries[0]);
        assert_eq!(row.title, "Hello World");
        assert_eq!(row.link, "http://example.com/1"); // Atom link 默认 rel=alternate
        assert_eq!(row.author, "Bob");
        assert!(row.published.contains("2026"));
        assert!(row.sort_key.is_some());

        // 第二条：无作者/无时间 → 兜底为空串与占位符。
        let row = build_entry_row("test", &feed.entries[1]);
        assert_eq!(row.title, "Second");
        assert_eq!(row.link, "http://example.com/2"); // 无 alternate 时取第一个 link
        assert_eq!(row.author, "");
        assert!(row.sort_key.is_none());
        assert_eq!(row.published, "—");
    }
}
