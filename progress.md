# Progress Log — Everyday

> 本文件仅保留**当前状态 + 核心决策（ADR）**。逐次会话的「已完成 / 测试结果 / 下一步」流水账已压缩；技术踩坑与 API 细节见 `findings.md`。

## 当前状态（2026-07-11）

- **v0.6.0 已发布**：tag `v0.6.0`，Mail Cache（envelope 缓存 + 并发 sync）实施完成。
- **代码全量 Review + 修复进行中**：基于 2026-07-11 的 caveman-style 全量 review（详见 commit history），按 🔴→🟡→🔵 顺序逐项修复并独立提交；每修一项必须 `cargo build` + `cargo clippy -D warnings` + `cargo test` + `cargo fmt --check` 全绿。
- **模块**：**7 个**外部集成模块 **mail / cal / rss / note / todo / bookmark / timeline** + `config` 均可用；note/todo/bookmark 支持本地 SQLite provider，**默认 local**；timeline 统一事件层（commit `2ce5055` + 修补 `045afa6` `9a3ef49` `8de8f26` `32f67c1`）；`mail list` v0.6.0 起走本地 envelope 缓存（`mail_cache.db`），staleness=15min 自动 sync，`--sync` 强制。
- **质量门禁**：`cargo build` ✅、`cargo clippy --all-targets -- -D warnings` ✅ 零警告、`cargo test` ✅ **200 passed**（v0.6.0 196 + review 期间新增 4 单测）；CI（ubuntu/macos/windows + aarch64 mac）全绿。
- **文档**：README + `skills/everyday-cli/*` 与代码一致；范围与定位以 `agents.md`「范围与定位」为权威说明（原 PRD.md 已移除）。

### 2026-07-11 — 全量代码 Review 修补流水（caveman-style）

按严重度逐项修复，每项独立 commit：

1. `fix(mail): PoolGuard::session returns Result instead of panicking` — 移除生产路径 `expect()` panic。
2. `fix(mail): PoolGuard Drop no longer panics when tokio runtime is down` — `Handle::try_current()` 探测。
3. `fix(timeline): eliminate double-unwrap on DST-boundary date parsing` — `.earliest()/.latest()` 替代 `.unwrap()`。
4. `fix(timeline): CalProvider::sync honors the window argument` — 修正 ADR 0002 契约。
5. `fix(output): JSON serialize failure no longer breaks --json contract` — `fallback_json` 兜底。
6. `fix(util): is_json() no longer scans std::env::args()` — 改用线程局部变量。
7. `fix(args): parse_simple_args no longer misclassifies negative numbers` — 负数 / 单破折号值保留为 flag 值。

## 核心决策时间线（ADR）

### 2026-07-08 — 基础架构定型
- 统一命令结构 `everyday <module> <action> [options]`；`Executor` trait + `ModuleRegistry` 解耦主程序与模块；`Output`(Text/Json/Table) 统一渲染；`AgentError` 统一错误并序列化 JSON。
- 多账户配置：`~/.config/everyday/config.toml` + keyring 存凭证（禁明文）。

### 2026-07-08 — 邮件模块（mail）
- IMAP/SMTP 走 `async-imap` + `lettre`（tokio-rustls `.compat()` 桥接）；文件夹递归列出 + 中文 IMAP UTF-7 解码（`select_folder` 智能匹配原始名/中文名）。

### 2026-07-09 — 日历模块（cal，CalDAV）
- 选型 `libdav` + `icalendar` + `hyper-rustls`（ring provider）；**跳过 `bootstrap_via_service_discovery`（国内无 DNS SRV）**，改 `find_context_path` 只做 well-known 重定向；QQ `/.well-known/caldav` 301 后覆盖 `base_url`；`cal list` 全量拉取 + 本地日期过滤（比服务端 time-range REPORT 可靠）。

### 2026-07-09 — CLI 帮助修复 & RSS 模块
- 子命令帮助：clap 内置 `--help` 在顶层拦截，改为 `main` 预扫描 raw args 分发 module/action 帮助。
- `rss`（feed-rs）：follow/list/unfollow/digest/fetch；`--json` 任意位置生效（修复 `trailing_var_arg` 吞标志）；并发抓取 + 最佳努力降级。

### 2026-07-10 — 范围收窄：移除 fs / net / sys
- 初版含 `fs`(文件搜索) / `net`(网页抓取/HTTP) / `sys`(系统监控) / 剪贴板等模块，经评审**整体移除**：这些封装的是代理可用 shell / `curl` / `fd` / `rg` 直接完成的通用能力，与「外部集成接口」定位不符。最终仅保留外部协议/状态/凭证类模块（mail/cal/rss，后扩展 note/todo）。详见 `findings.md`「架构决策：移除 fs / net / sys 模块」。

### 2026-07-10 — note / todo 模块 + 共享 notion-client
- `note`（Notion 笔记）六动作 + `list`；`todo`（Notion 待办）六动作；底层共享 `notion-client`（429 退避重试）。

### 2026-07-10 — CI / Release / v0.1.0
- `.github/workflows/ci.yml`（三平台 + aarch64 macOS，clippy `-D warnings` + `cargo fmt --check`）、`release.yml`（tag `v*` 触发，三平台 + aarch64 二进制）；发布 v0.1.0。

### 2026-07-10 — 移除过时的 PRD.md
- PRD 仍描述已删除的 fs/net/sys/剪贴板，与现实脱节；`git rm` 并清理全仓引用，范围改以 `agents.md` 为准（commit `fc14584`）。

### 2026-07-10 — bookmark 模块（书签：Notion + 本地 SQLite）
- 新增 `bookmark` 模块：动作 `init-db` / `add`（--url --title --tags）/ `list`（--tag 过滤）/ `login`（仅 notion）。
- 双 provider 对齐 note/todo：`local`（默认，SQLite，表 `bookmarks` + 关联表 `bookmark_tags` 支持按标签精确过滤）与 `notion`（`init-db` 在 Notion 建库 Title/URL/Tags，add/list 走 `notion-client` 强类型映射）。
- 配置：`[[bookmark.accounts]]` + `default_account.bookmark`；keyring service `everyday/bookmark/<account>` 存 token；`default_database_id` 由 `init-db` 回填。
- 沿用既有 ADR：不新增 `AgentError` 变体、禁 `unwrap`、用 `toml::Value` 局部编辑 config。

### 2026-07-10 — 发布 v0.2.0
- 自 v0.1.0 以来的增量：`feat(todo)` Notion 待办模块 + 共享 notion-client SDK（commit `a721f5c`）、`fix(todo)` Status 改为 select 修复 Notion 过滤、`ci` 增加 aarch64-apple-darwin 到 CI + release 矩阵、移除过时 PRD.md、精简 progress/findings/task_plan 历史。
- 版本号 `0.1.0 → 0.2.0`（Cargo.toml + Cargo.lock 由 cargo 自动更新）；tag `v0.2.0` 触发 release workflow 构建三平台 + aarch64 macOS 预编译二进制。

### 2026-07-10 — 发布 v0.3.0
- 自 v0.2.0 以来的增量：`feat(note,todo)` 新增本地 SQLite provider（sqlx，`provider = "local"`/`sqlite`）、`feat(note,todo)` **默认 provider 由 notion 改为 local**（config init/示例/文档同步；显式 `provider = "notion"` 旧配置向后兼容）。
- 版本号 `0.2.0 → 0.3.0`（Cargo.toml + Cargo.lock 由 cargo 自动更新）；tag `v0.3.0` 触发 release workflow 构建三平台 + aarch64 macOS 预编译二进制。
- 质量门禁：build ✅ / clippy `-D warnings` 零警告 ✅ / `cargo test` 126 passed ✅。

### 2026-07-10 — 重构：清理 dead_code + note 接入共享 notion-client
- 移除 `main.rs` 的 crate 级 `#![allow(dead_code)]`（该抑制原本为「预留公共 API」而加，现模块已齐备会掩盖死代码），恢复 clippy 对死代码的正常检查。
- 删除确认的死代码：`Config::save`/`save_to`、`AgentError::NotImplemented` 变体、`ModuleRegistry::module_names`、`Output::json` 构造器，以及 `notion_client` 中带 `#[allow(dead_code)]` 且从未读取的 `token` 字段；相应测试同步调整（保留 Config 序列化 roundtrip 与 JSON 渲染覆盖）。
- `note` 模块接入 `src/notion_client.rs` 共享 `NotionClient`：删除其自建的 `build_client`/`notion_request`/`api_get`/`api_post`/`api_patch` 与 `NOTION_API`/`NOTION_VERSION` 常量，`fetch_all_blocks` 改为接收 `&NotionClient`，所有请求走 `get`/`post`/`patch` + 分页查询。行为不变（401/403→Auth、其它→Network、429 自动退避重试），`note read` 的 block 递归聚合改用 `&NotionClient`。
- 消除原 ADR「note 暂不复用 notion_client（择机去重）」的偏差，mail/cal/rss 之外两个 Notion 模块现在共用同一底层 SDK。
- 质量门禁：`cargo build` ✅、`cargo clippy --all-targets -- -D warnings` ✅ 零警告、`cargo test` ✅ 113 passed。

### 2026-07-10 — bookmark 文档对齐（config / README / skills）
- 补齐 bookmark 模块的文档，使其与代码（commit `79922f6`）一致：
  - `config.example.toml`：`[default_account]` 加 `bookmark = "personal"`；新增 `[[bookmark.accounts]]`（local provider，注释给出 Notion 备选）。
  - `README.md` + `README_ZH.md`：概览行、bookmark 小节（命令表 + 选项 + 标签解析说明 + local/Notion provider 说明）、配置示例、使用示例、目录树加 `bookmark.rs`、实现状态表加 bookmark 行。
  - `skills/everyday-cli/references/COMMANDS.md`：实现状态表 + 完整 `## bookmark` 小节、配置示例、keyring service 命名行补 `everyday/bookmark/personal`。
  - `skills/everyday-cli/SKILL.md`：frontmatter description、`Modules:` 列表、`Modules.` 描述三处均补 `bookmark`。
- 纯文档改动；门禁仍全绿：build ✅ / clippy `-D warnings` 零警告 ✅ / 137 tests ✅。

### 2026-07-11 — 发布 v0.4.0
- 自 v0.3.0 以来的增量：
  - `feat(bookmark)`：新增 bookmark 模块，双 provider（local SQLite 默认 + Notion），commit `79922f6`。
  - `docs(bookmark)`：配置示例 / README(中英文) / skills 文档对齐，commit `ca40fbe`。
  - 模块分层 `modules` / `shared` / `util` 去重（`d532f3d`）；新增 `Justfile` 开发流程（`3a1412a` + `ea59506` + `92f8a83`）；README 国际化英文为默认（`5944c8f`）；CI 加 `cargo fmt --check` 门槛（`1a5704e`）。
- 版本号 `0.3.0 → 0.4.0`（Cargo.toml；Cargo.lock 由 `cargo build` 自动同步）。
- 质量门禁：build ✅ / clippy `--all-targets -D warnings` 零警告 ✅ / `cargo test` 137 passed ✅。
- release commit `ca40fbe` 之后 bump 版本并打 tag `v0.4.0`，推送 `origin`（GitHub）触发 release workflow 构建三平台 + aarch64 macOS 预编译二进制。cnb 镜像不推。

### 2026-07-11 — Timeline 统一事件层（commit `2ce5055`）
- 按 `CONTEXT.md`（领域术语表）+ `docs/adr/0001`–`0009`（9 个架构决策）落地。
- 核心架构：append-only event log + 纯 pull 模型 + `TimelineProvider` 独立 trait + 各模块暴露 `fetch_for_timeline(window)`。
- 数据库：`~/.config/everyday/timeline.db`（events + sync_state，自然键 `(source, COALESCE(account,''), ref_id, event_type, timestamp)` 唯一索引）；`~/.config/everyday/ops-log.db` 记录 notion 账户 CLI 操作。
- 6 个 source adapter：mail（IMAP 拉取）/ cal（CalDAV，**窗口刷新**例外，前看 7 天）/ rss / todo local / note local / bookmark local。
- Sync 编排器：按 source 分组并行（`futures::join_all`），同 source 串行；best-effort 失败 provider 水位不变，下次重试。
- 查询：`today` / `yesterday` / `week`（周一-周日）/ `month` / 自定义 `--from/--to`；`--source/--account/--limit/--sync/--since` flags；UTC 存储 + 本地时区查询 + 本地时间显示。
- AOP hook：`main.rs::run()` 执行成功后调 `ops_log::maybe_log_op()`，仅记录 `todo/note/bookmark` 的 notion 账户写操作；模块零侵入。
- 顺手修 3 个 bug：
  1. `gen_id` 同纳秒撞 ID（影响所有 caller；timeline 首个高密度触发点）——加 `AtomicU64` 计数器保唯一。
  2. `query_events` LIMIT 占位符缺 `?` 前缀导致 `LIMIT {idx}` 当字面（测试期望 2 行返回 1 行）——改字面整数。
  3. `idx += 1` 死赋值（clippy `unused_assignments`）—— 删。
- 质量门禁：build ✅ / clippy `-D warnings` 零警告 ✅ / `cargo test` 173 passed（+36 全为 timeline）✅ / `cargo fmt --check` clean ✅。

### 2026-07-11 — Timeline 4 处修补（同时清理 3 条 Notion 测试残留）

发布 v0.5.0 前对 timeline 做端到端实测,发现 4 类缺陷(全部提交修掉):

| Commit | 修补 | 问题 |
|---|---|---|
| `045afa6` | 新增 `OpsLogProvider` | notion todo/note/bookmark 写入只入 ops-log,从未进入 timeline.events,`timeline list` 看不到 |
| `9a3ef49` | ops-log 解析 `Output::Text` | 默认文本模式 AOP 完全不触发,只有 `--json` 才落 ops-log |
| `8de8f26` | `--since` 在 query 路径生效(支持日期与 30m/2h/1d 相对时长,保留 sub-day 精度) | help 写有,但 query 路径 silent 忽略 |
| `32f67c1` | todo 加 `delete` action(notion + local;归档前 GET 标题,让 ops-log delete 行带 title) | 没有 CLI 删除路径,只能 Notion UI |

清理:Notion 上 3 条 opslog-test / timeline-opslog-test / textmode-test-after-fix 已通过 `everyday todo delete` 归档。  
ops-log.db 留 3 条历史 delete 行(标题空,归档前为空记录,无影响)。  
timeline.db 重 sync 后 6/6 providers,75 events(mail 60 + cal 9 + rss 7 - 重叠去重 - 配 0;opslog todo 6 + opslog note 0 + opslog bookmark 0)。

最终质量门禁:build ✅ / clippy `-D warnings` 零警告 ✅ / `cargo test` **181 passed**(+8 新单测)/ `cargo fmt --check` clean ✅。

**可以按 runbook 发版 v0.5.0**:`Cargo.toml` 0.4.0 → 0.5.0 + 当前状态行 + Phase 9 → `progress.md` 当前状态行 + ADR → `chore: release v0.5.0` → tag `v0.5.0` → `git push origin master && git push origin v0.5.0`(推 GitHub,绝不推 cnb 镜像)。

### 2026-07-11 — Mail Cache（v0.6.0，待发版）

按 `CONTEXT.md` §Mail Cache + `docs/adr/0010`–`0013` 落地。`mail list` 从直连 IMAP 改为本地 envelope 缓存 + 并发跨 folder sync。

- **核心架构**：
  - `src/modules/email_cache.rs`：`mail_cache.db`（`~/.config/everyday/`）双表。`envelopes` 主键 `(account, folder, uid)`，扩展字段 `date/from_addr/subject/flags/message_id/size/to_addr/fetched_at`；索引 `(account, date DESC)` 与 `(account, folder)`。`folder_state` 主键 `(account, folder)`，存 `uid_validity/max_uid/last_sync_at`。
  - `src/modules/email_pool.rs`：M=4 IMAP session 池 + `Arc<Semaphore>`；`PoolGuard` 借用归还，`invalidate()` 标 dirty 不归还。
  - `src/modules/email.rs::mail_list` 改造：开 cache → staleness 检查（任一 folder `last_sync_at > 15min` 或无水位 → sync）→ 必要时并发 sync（`sync_folders_concurrent` 用 `futures::join_all` 跨 folder）→ 查本地 envelope → 渲染表格。
- **关键 sync 流程（单 folder）**：`SELECT folder` 拿 `uid_validity` → 比对本地 → 不一致则 `clear_folder` 回退全量 → `UIDSEARCH UID <max_uid+1>:*`（首次 = `UID 1:*`）→ `UID FETCH (UID ENVELOPE FLAGS RFC822.SIZE)` → 事务 `upsert_envelopes` 写 envelope + 前进水位（ADR 0012 强一致）。
- **保持不变**：`mail search` / `mail read` / `mail send` 仍直连 IMAP（与缓存正交）。`fetch_for_timeline` 仍走 server（与 Timeline 现有实现兼容）。
- **明确边界**：flags 是 sync 时刻快照（最坏 15 分钟滞后，F1）；K1 只追加不清理（数据库增长无界，10 万封 ≈ 30MB 可接受）。
- **新 flag**：`--sync` 强制立即 sync（无视 staleness）。无 `--no-cache` / `--full` 等 flag（KISS）。
- **单测**（+15）：4 个 staleness 边界（阈值恰 = 不 stale、+1 = stale、最近 60s/1000s 状态）+ 1 个 pool capacity + 2 个 parse_rfc3339 + 8 个 SQL 集成（upsert 写 + 水位前进、空 batch 仅前进 last_sync、upsert on conflict、clear_folder、UIDVALIDITY 失效模拟、unread 过滤、K1 ghost envelope 留存、date desc + limit）。
- **质量门禁**：build ✅ / clippy `-D warnings` 零警告 ✅ / 196 tests passed (+15) ✅ / fmt clean ✅。
- **待发版 v0.6.0**：`Cargo.toml` 0.5.0 → 0.6.0 + 当前状态行 + Phase 10 → `progress.md` 当前状态行 + ADR → `chore: release v0.6.0` → tag `v0.6.0` → `git push origin master && git push origin v0.6.0`（推 GitHub，绝不推 cnb 镜像）。
