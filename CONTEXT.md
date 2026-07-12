# CONTEXT.md — Everyday Modules

> 领域术语表。仅定义概念，不涉及实现细节。
> 决策记录见 `docs/adr/`。
>
> 每个模块独立成节：Timeline / Mail Cache / 其他后续模块。

## 核心概念

### Timeline
统一的事件存储层。将 Mail / Calendar / RSS / Todo / Note / Bookmark 等模块产生的事件标准化为**只追加日志（append-only log）**，提供统一查询。Timeline 回答的是"过去发生了什么"，而非"当前状态是什么"。当前状态由日志顺序重放派生。

### Timeline Event
Timeline 中的一条不可变记录，表示"某事在某时刻发生了"。一旦写入永不修改（除非同步去重或显式清理）。一个事件的标识由自然键 `(source, ref_id, event_type, timestamp)` 唯一确定——同一个 `ref_id` 可对应多条事件（如一个 todo 的 created 与 completed 是两条独立事件）。

### Source
事件的来源模块。稳定枚举值：`mail` / `cal` / `rss` / `todo` / `note` / `bookmark`。不编码账户信息。

### Account
事件所属的来源账户名（如 `work` / `personal`）。Schema 一等列，纳入自然键。RSS 无账户概念，该列为 NULL。本地单账户模块（todo/note/bookmark）取配置中的账户名。

### 自然键
`(source, COALESCE(account, ''), ref_id, event_type, timestamp)`。用于同步幂等：相同窗口重复同步不产生重复行。跨账户的相同 `ref_id` 因 `account` 不同而视为不同实体。

### Event Type
事件的语义类型，由各 source 自定义。同一 `ref_id` 的不同 `event_type` 代表该实体的不同生命周期时刻。

各 source 的 `event_type` 约定：

| Source | event_type | timestamp 取值 | 同步模式 |
|---|---|---|---|
| `mail` | `received` / `sent` | 邮件 Date header | 追加 |
| `rss` | `published` | 文章发布时间 | 追加 |
| `bookmark` | `added` | 收藏时刻 | 追加 |
| `todo` | `created` / `started` / `completed` / `deleted` | 状态切换时刻 | 追加 |
| `note` | `created` / `updated` | 创建 / 更新时刻（`updated_at`） | 追加 |
| `cal` | `scheduled` | 会议开始时间 | **窗口刷新** |

### 同步模式（Sync Mode）
- **追加（Append）**：mail / rss / bookmark / todo / note。按自然键 `(source, ref_id, event_type, timestamp)` 幂等写入，重复同步不产生重复行。
- **窗口刷新（Window Refresh）**：仅 `cal`。同步时先删除当前窗口内 `source='cal'` 的旧行，再插入当前快照。理由：日历事件是"未来投影"而非"已发生动作"，reschedule / delete 会改变投影内容，强行塞进纯 append 模型会产生同一会议的多条幽灵记录。

### 软删除（Soft Delete）
Todo 的 `deleted` 事件类型作为软删除标记。查询时默认过滤已标记 `deleted` 的 `ref_id`。append-log 不物理撤回已写事件，仅以 `deleted` 事件表示删除意图。

### TimelineProvider
各模块实现的 trait，向 Timeline 吐出事件。与 `Executor`（响应用户命令）正交——一个模块可同时实现两者。Provider 无状态：只接收编排器传入的 `TimeWindow`，返回该窗口内的事件快照与同步模式。水位（last_sync）由编排器在 timeline.db 的 `sync_state` 表中管理，不由 provider 持有。

### Sync 编排器
Timeline sync 的协调层。职责：从 `sync_state` 读取各 (source, account) 的水位 → 构造 `TimeWindow { from: last_sync, to: now }` → 遍历 `TimelineProviderRegistry` 调用各 provider 的 `sync()` → 按返回的 `SyncMode`（Append / WindowRefresh）写入 events 表 → 更新水位。

### TimelineProviderRegistry
独立于 `ModuleRegistry`（后者用于 `Executor` 命令分发）。构建时注入 `Arc<Config>`，遍历 config 中配置的账户，为每个 (source, account) 构造一个 provider 实例。

### Pull 模型
所有 source 统一由 sync 编排器拉取，包括本地模块（todo/note/bookmark）。本地模块不在写操作时 push 事件——它们的 provider 在 sync 时查询各自 SQLite 表（按 `created_at`/`updated_at` 增量）转为事件。模块间无横向依赖：timeline 作为上层消费者拉取，方向单向。

### 查询与同步的分离
`timeline` 查询只读 SQLite（毫秒级，符合 < 100ms 冷启动预算），不自动触发 sync。需要新数据时显式 `everyday timeline sync`。查询支持 `--sync` flag 主动触发一次 sync 再查。

### 首同步水位
`sync_state` 表初始为空时，首次 sync 的 `from = now - 30天`（默认回看窗口）。用户可 `timeline sync --since 2026-01-01` 覆盖。不做全量历史拉取（mail 全量极慢）。`sync_state` 存 `first_sync_done` 标志。

### Cal 窗口边界
cal 的窗口刷新模式窗口 = `[last_sync, now + 7天]`（前看 7 天）。实用例外：`timeline today` / `timeline week` 需显示未来几天的安排。未来事件按 timestamp 过滤，不影响纯过去查询（`timeline yesterday` 不命中未来事件）。

### 时间存储与时区
事件 `timestamp` 存 UTC（RFC3339 带 `Z` 结尾），保证字典序 = 时间序（带不同时区偏移的 RFC3339 字符串字典序与时间序不一致，故必须统一存 UTC）。查询时用 `chrono::Local` 算本地日期边界（如 `timeline today` 在杭州 22:00 执行 → 本地 `[2026-07-11T00:00+08, 23:59:59+08]` → 转 UTC `[2026-07-10T16:00Z, 2026-07-11T15:59:59Z]` 查询）。输出时把 UTC timestamp 转回本地时间显示。

### 数据库位置
固定 `~/.config/everyday/timeline.db`，单库聚合所有 source/account 事件。不暴露多账户路径。首版不实现 `[timeline] db_path` 覆盖配置。

### Schema
- `events` 表：`id`（代理主键 UUID 短码）、`source`、`account`（NULL for rss）、`event_type`、`timestamp`（RFC3339 UTC）、`title`、`summary`、`ref_id`、`metadata`（JSON）、`created_at`（写入时刻）。
- `ux_events_natural` 唯一索引：`(source, COALESCE(account,''), ref_id, event_type, timestamp)`——幂等去重，COALESCE 处理 rss 的 NULL account。
- `ix_events_time_source` 索引：`(timestamp, source)`——覆盖主查询模式。
- `sync_state` 表：`(source, COALESCE(account,''))` 主键、`last_sync`、`first_sync_done` 标志。

### ref_id
事件引用的实体在来源系统中的稳定标识（如邮件 `<account>:<IMAP UID>`、日历 VEVENT UID、todo 的本地 id）。用于跨多条事件关联同一实体的生命周期。

### Provider 范围与 Notion ops-log
- **local provider**（todo/note/bookmark 默认）：TimelineProvider 直接查模块自己的 SQLite 表（按 `created_at`/`updated_at` 增量），毫秒级。
- **notion provider**：不查 Notion API（无增量历史、限流、状态变更时间丢失）。改为从**本地 ops-log**（`~/.config/everyday/ops-log.db`）拉取——CLI 执行 notion 账户动作时记录的操作审计日志。
- **明确限制**：notion.so 网页端 / 其他客户端的修改不进 ops-log，Timeline 看不到。仅捕获 CLI 发起的操作。

### Ops-log（操作审计日志）
统一库 `~/.config/everyday/ops-log.db`，记录 CLI 对 notion 账户执行的写操作。Schema：
- `ops_log` 表：`id`（自增）、`module`（todo/note/bookmark）、`account`、`action`（add/complete/start/delete/create/update）、`ref_id`、`title`、`metadata`（JSON）、`occurred_at`（RFC3339 UTC）。
- 索引 `ix_ops_module_account_time(module, account, occurred_at)`。

Timeline 的 notion provider 查 `SELECT * FROM ops_log WHERE module=? AND account=? AND occurred_at > ?`，映射为事件。

### 各 source 字段映射
| source | event_type | title | summary | ref_id | metadata |
|---|---|---|---|---|---|
| `mail` | `received`/`sent` | Subject | `From/To` 前 200 字符 | `<account>:<UID>` | `{from,to,folder,message_id}` |
| `cal` | `scheduled` | iCal SUMMARY | `LOCATION + DTSTART-DTEND` | VEVENT UID | `{calendar,location,start,end,attendees}` |
| `rss` | `published` | 文章标题 | 摘要前 200 字符 | guid/link | `{feed,url,link,author}` |
| `todo` | `created`/`started`/`completed`/`deleted` | todo title | `""` | todo id | `{status,due,priority}` |
| `note` | `created`/`updated` | note title | `""` | note id | `{props:{...}}` |
| `bookmark` | `added` | bookmark title | url | bookmark id | `{url,tags:[...]}` |

### Ops-log AOP 写入
ops-log 写入与模块脱钩，采用 dispatch 层 hook（`main.rs::run()` 中 `module.execute()` 返回后调用 `ops_log::maybe_log_op()`）。逻辑集中在 `src/ops_log.rs`：
1. 仅 `todo`/`note`/`bookmark` 模块的写操作记录（`list`/`search`/`read`/`login`/`init-db` 等跳过）。
2. 仅 `provider = "notion"` 的账户记录（local 账户的 timeline provider 直接拉 SQLite）。
3. 从 Output 的 JSON 提取 `id`（→ ref_id）和 `title`（可能缺失，取空）。
4. 写入失败不阻断用户命令（`let _ =` 吞错，stderr 警告）。

### 本地 provider 的事件粒度
本地 provider 从"当前态快照"拉取，非完整转移历史。降级语义：
- **todo**：`created`（`created_at`）+ 当前状态映射的事件（`updated_at` 变化时生成，如 `completed`）。多次转移合并为一条最新态事件（Todo→In Progress→Done 间只生成 `completed`）。需给 `todos` 表加 `updated_at` 列。
- **note**：`created`（`created_at`）+ `updated`（`updated_at`）。多次更新合并为一条。
- **bookmark**：`added`（`created_at`）。
- **删除**：当前无 delete action，暂不支持。将来加 delete 时改为软删除（`deleted_at` 列），provider 查 `WHERE deleted_at > last_sync`。
- **notion 账户**无此降级——ops-log 在执行时记录每次转移，粒度完整。

### Sync 执行模型
- **Best-effort + 逐 provider 水位**：每个 provider 独立 try/catch。成功的更新 `sync_state` 水位，失败的跳过（水位不变，下次 sync 重试该窗口）。一个坏源不阻塞其他源。sync 整体返回成功，输出标注失败项。
- **按 source 分组并行**：不同 source 的 provider 并行执行（`futures::join_all`），同 source 的多账户串行（避免同服务器限流）。本地 provider（todo/note/bookmark）毫秒级，串行无感。
- **输出**：文本模式逐 provider 统计（events 数 / 失败原因）；JSON 模式结构化 `{providers_total, providers_ok, providers_failed, events_synced, details:[...]}`。
- **CLI**：`timeline sync`（全量）、`--source mail,cal`（过滤）、`--since 2026-06-01`（覆盖回看窗口）。

### 文件结构
```
src/
├── modules/
│   ├── mod.rs               # +1 行注册 TimelineModule
│   ├── timeline.rs          # Executor impl（响应 everyday timeline 命令）
│   └── timeline/            # timeline 内部子模块
│       ├── mod.rs           # TimelineProvider trait + SyncMode + TimeWindow
│       ├── store.rs         # timeline.db 读写（events / sync_state 表）
│       ├── orchestrator.rs  # sync 编排器（遍历 providers、水位管理、并行）
│       └── providers.rs     # 各 source 的 provider adapter（调各模块 fetch_for_timeline）
├── ops_log.rs               # AOP hook（dispatch 层 ops-log 写入）
```

### Provider adapter 模式
各模块暴露 `pub async fn fetch_for_timeline(window: &TimeWindow) -> Result<Vec<TimelineEvent>>` 数据获取函数。timeline/providers.rs 为每个 source 写 adapter，调用各模块的 `fetch_for_timeline` 并转换为 `TimelineEvent`。依赖方向：timeline → 各模块（单向），各模块不依赖 timeline 类型定义。

### TimelineModule 注册
TimelineModule 实现 `Executor`，注册到 `ModuleRegistry`。`name()` = `"timeline"`，`actions()` = `[today, yesterday, week, month, sync]`（无 action = today）。不在 TimelineProviderRegistry 里——它是消费者（Executor），不是数据源（Provider）。timeline 走正常 ModuleRegistry 分发，无需 main.rs 特殊拦截（与 config 模块不同）。

### 初始化与空状态
- timeline.db 懒创建：首次查询或 sync 时 `connect()` + `create_if_missing`（复用 `local.rs::connect`）。
- 空状态（未 sync）：`timeline today` 返回空列表，不报错。文本 "no events"；JSON `[]`。
- 首次 sync：所有 provider 的 `last_sync` 缺失 → `from = now - 30天` → 全量回填近 30 天。

### Sync
从各数据源拉取事件并写入 Timeline 日志的过程。同步以增量窗口进行，幂等重放（相同窗口重复同步不产生重复行，靠自然键约束）。

### 查询 CLI
- `everyday timeline`（无 action）= `today`。
- 预设：`today` / `yesterday` / `week`（周一到周日，ISO 8601）/ `month`（自然月 1 日到月末）。边界按本地时区 00:00–23:59:59，转 UTC 后查询。
- 自定义范围：`--from YYYY-MM-DD` / `--to YYYY-MM-DD`（本地日期，00:00 / 23:59:59）。
- 过滤：`--source mail,todo`（逗号分隔枚举）、`--account work`（单值过滤，复用全局 flag 语义为"过滤显示"而非"选操作账户"）。
- `--limit N`（默认 100，0 = 无上限）。
- `--sync`：查询前触发一次 sync 再查。

### 查询输出
- 排序：timestamp 降序（最新在上）。
- 文本表格列：`TIME`（本地 `MM-DD HH:MM`）/ `SOURCE` / `TYPE` / `TITLE`。summary 不进表格。
- JSON 数组：每元素含 `id` / `source` / `account` / `event_type` / `timestamp`（UTC RFC3339）/ `title` / `summary` / `ref_id` / `metadata`。

---

## Auth

> 凭据与认证层。设计决策见 `docs/adr/R013` `R014` `R015`。

### Credential（凭据）
用于向外部服务认证某账户的秘密（密码或 API token）。只存于 OS keyring，永不写入配置文件或日志。

### AuthStrategy（认证策略）
按 (module, account) 派生出的凭据分类，决定如何存储与验证：
- `Password`：`mail` / `cal`。keyring user = `account.username`；验证走 IMAP / CalDAV 真实登录。
- `Token`：`note` / `todo` / `bookmark` 且 `provider = "notion"`。keyring user = `"token"`；验证走 `notion_client`。
- `None`：`note` / `todo` / `bookmark` 的 `local`/`sqlite` provider，以及 `rss`。无凭据，无需存储或验证。

### auth 模块
顶层命令 + 共享模块，独占凭据的完整生命周期（store / get / delete / verify）。各模块改调 `auth::get_credential` 而非自行读 keyring。

### verify（认证）
证明已存凭据有效的行为——连接外部服务（IMAP/CalDAV 登录、Notion API 调用）。与"凭据存储"正交：`auth login` 默认只存，`verify` 是显式可选步骤（`--verify` flag 或独立 `auth verify` 动作）。

### not_required
`verify` / `list` 的一种状态，表示该账户 provider 无需凭据（local/sqlite、rss），故无可存、无可验。

---

## Mail Cache

> `mail` 模块的本地缓存层。独立于 Timeline.db。
> 决策见 ADR [M002](docs/adr/M002-imap-connection-pool.md)、[M003](docs/adr/M003-envelope-cache.md)、[M004](docs/adr/M004-uid-watermark-sync.md)、[M005](docs/adr/M005-staleness-auto-sync.md)。

### Mail Cache

`mail` 模块的本地 envelope 缓存层。位置 `~/.config/everyday/mail_cache.db`。目的：让 `mail list` 默认查本地（毫秒级），仅 sync 时走 IMAP 网络。

### Envelope

一封邮件的摘要信息，用于列表展示与基础过滤：`(uid, folder, account, date, from, subject, flags, message_id, size, to, fetched_at)`。**不含 body / attachments**——读完整邮件仍走 IMAP `BODY[]`（见 `mail read`）。Envelope 是 IMAP `ENVELOPE` fetch 字段的本地映射（IMAP envelope 不含 flags / size，此处扩展）。

### 主键语义

`(account, folder, uid)` 复合主键。IMAP UID 是 **folder-scoped**——同一封邮件在不同文件夹有不同 UID；同一 (account, folder) 内 UID 单调递增（除非 UIDVALIDITY 变更）。复合主键准确表达这一约束。

### UID Watermark（水位）

每文件夹在 `folder_state` 表存的水位元数据 `(account, folder, uid_validity, max_uid, last_sync_at)`。Sync 时用 `UIDSEARCH UID <max_uid+1>:*` 取新增邮件 UID——RFC 3501 推荐的增量方式。`max_uid = 0` 表示首次 sync，等价全量（`UIDSEARCH UID 1:*`）。

### UIDVALIDITY 变更

IMAP 文件夹重建（邮件被 server 端批量删除 / 索引重建）时 `UIDVALIDITY` 字段变化，旧 UID 被复用为不同邮件。**每次 sync 前必须 SELECT 拿当前 UIDVALIDITY 与本地比对**，不一致则清空该 folder 的水位与 envelope，回退到全量 SELECT。RFC 3501 §2.3.1.1 强制。

### Staleness

`folder_state.last_sync_at` 距今的时长。默认 `mail list` 若任一目标 folder staleness > **15 分钟**（写死，无 flag），自动触发一次 sync 再查本地。`--sync` flag 强制立即 sync，无视 staleness。阈值不暴露为配置项。

### flags 缓存语义

envelope 的 `flags` 字段是 **sync 时刻的快照**。用户在 web 端 / 其他客户端改 flags 后，本地缓存最坏 15 分钟滞后（直到下次 sync）。`mail list --unread` 反映 sync 时的状态——明确边界，可接受。**不做实时 flags 重拉**（破坏"零网络 list"目标）。

### Ghost Envelope（幽灵邮件）

K1 清理策略：sync 只追加不删除。Server 端被删除 / 跨文件夹移动的邮件，本地 envelope 仍留着。默认 `mail list --limit 20` 按日期排序，幽灵邮件通常已被新邮件挤出 limit，不影响日常体感。数据库无界增长（10 万封约 30MB），未来可加 `mail cache gc` 命令手动清理。

### Search 不走缓存

`mail search --query Q` 仍走 IMAP 直连（`SEARCH TEXT "Q"`），不读本地缓存。原因：IMAP SEARCH 支持 subject + body + header 多字段搜索；本地 LIKE 仅能搜 subject/from，语义不对等。与 `mail list` 行为不对称，文档中明确。

### Read 不走缓存

`mail read <uid>` 走 IMAP 直连 `BODY[]` 拉完整邮件。原因：完整邮件含附件 / multipart，本地不缓存。Envelope 缓存仅服务 list 场景。

### Send / Login 不走缓存

`mail send` / `mail login` 与缓存无关，按需走 SMTP / keyring。

### 连接池与并发

固定大小 IMAP session 池（M=4）+ `tokio::Semaphore` 分发到 N 个文件夹。每次 sync 启动时建 4 个 IMAP session（共享 keyring 密码），文件夹 sync 抢占信号量、复用空闲 session。降低 TLS 握手 + IMAP LOGIN 开销，同时避免触发服务器并发连接 ban。4 是经验值，不通过 flag / config 暴露。

### 数据库 Schema

```
envelopes(
    account    TEXT NOT NULL,
    folder     TEXT NOT NULL,
    uid        INTEGER NOT NULL,
    date       TEXT NOT NULL,        -- RFC3339 UTC
    from_addr  TEXT NOT NULL,        -- mailbox@host
    subject    TEXT NOT NULL,        -- decoded MIME
    flags      TEXT NOT NULL,        -- IMAP flags space-separated
    message_id TEXT,                 -- RFC 5322 Message-ID header (nullable)
    size       INTEGER,              -- RFC822.SIZE in bytes (nullable)
    to_addr    TEXT,                 -- first To recipient mailbox@host (nullable)
    fetched_at TEXT NOT NULL,        -- RFC3339 UTC
    PRIMARY KEY (account, folder, uid)
)
CREATE INDEX ix_envelopes_account_date ON envelopes(account, date DESC);

folder_state(
    account       TEXT NOT NULL,
    folder        TEXT NOT NULL,
    uid_validity  INTEGER NOT NULL,
    max_uid       INTEGER NOT NULL DEFAULT 0,
    last_sync_at  TEXT NOT NULL,    -- RFC3339 UTC
    PRIMARY KEY (account, folder)
)
```

### 与 Timeline 的边界

mail_cache.db **独立**于 timeline.db，不跨数据库 JOIN。Timeline 的 mail provider（`fetch_for_timeline`）仍走 IMAP 直连 `SINCE <date>` 拉窗口内邮件（与本缓存层正交）。两个本地存储层服务于不同目标：

- **mail_cache.db**：agent 调用 `mail list` 时的快速响应层（envelope 摘要）。
- **timeline.db**：跨模块事件聚合层（envelope → Timeline event 投影）。

未来可选：将 mail_cache 投影到 Timeline 的 `MailProvider`（避免 `fetch_for_timeline` 重连 server）。不在本 ADR 范围。
