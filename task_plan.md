# Everyday 开发计划

**项目：** Everyday — The Rust-powered hands for your AI Agent
**范围：** 以 `agents.md`「范围与定位」节为权威说明（原 PRD.md 已移除）
**启动时间：** 2026-07-08
**当前状态：** v0.3.0 已发布；5 个外部集成模块（mail/cal/rss/note/todo）+ config 均可用，note/todo 支持本地 SQLite provider 且默认 local；`cargo build` / `cargo clippy --all-targets -- -D warnings` / `cargo test` 全绿（126 passed）。

---

## 总体目标

打造高性能、内存安全的本地 CLI 工具集，作为 AI Agent 的"数字双手"。统一命令结构 `everyday <module> <action> [options]`，支持 Text / JSON 双输出模式，JSON 为 AI 交互主模式。

---

## 阶段规划

### Phase 1: 项目地基与文档 [complete]
基础架构：`Cargo.toml`（包名 `everyday`，edition 2024）、`agents.md` 协作规范、`src/` 骨架（cli/config/error/output/modules）。

### Phase 2: 配置系统（多账户） [complete]
`config.rs` 加载与多账户合并、`everyday config set/get/list/path/init`、凭证走 keyring（config 只存元数据）、`config.example.toml`。

### Phase 3: 核心抽象 [complete]
`AgentError` 统一错误 + JSON 格式；`Output` 结构体（Text/Json/Table）；`Executor` trait + `ModuleRegistry`。

### Phase 4: CLI 框架 [complete]
`cli.rs`（clap derive，扁平结构 + 子命令帮助预扫描）、`main.rs`（解析 → 配置 → 查找 → 执行 → 渲染 → 退出码）。

### Phase 5: 模块骨架 [complete]
各模块实现 `Executor`；未实现/未知动作返回 `NotImplemented`/`UnknownAction`；注册到 `ModuleRegistry`。
> 注：初版曾含 `system`/`network`/`fs` 骨架，已于 2026-07-10 整体移除（见 `findings.md`「架构决策：移除 fs / net / sys 模块」）。

### Phase 6: 模块实现 [complete]
- `mail`（IMAP list/read/search + SMTP send + keyring login；文件夹递归 + IMAP UTF-7 中文解码）[2026-07-08]
- `calendar`（CalDAV：login/calendars/list/add/delete，libdav+icalendar）[2026-07-09]
- `rss`（feed-rs：follow/list/unfollow/digest/fetch）[2026-07-09]
- `note`（Notion 笔记/知识库：login/search/create/read/append/update/list）[2026-07-10]
- `notion-client` 共享 SDK + `todo`（Notion 待办：login/init-db/list/add/start/complete）[2026-07-10]

### Phase 7: 构建、测试、文档、发布 [complete]
全模块 `cargo build` / `clippy` / 单测 + 集成测试全绿；README + skills 文档与代码一致；CI（三平台 + aarch64 macOS）+ release workflow；**v0.3.0 已打 tag 发布**。

---

## 关键设计决策

| 决策点 | 选择 | 理由 |
|---|---|---|
| 包名 | `everyday` | 项目约定 |
| 异步运行时 | `tokio` | 生态成熟 |
| CLI 解析 | `clap` (derive) | 类型安全 |
| 错误处理 | `thiserror` + `Result<T, AgentError>` | 统一错误类型，易序列化 |
| 配置格式 | TOML | 人类可读 |
| 凭证存储 | `keyring`（service=`everyday/<module>/<account>`） | 安全红线：禁明文，Token 绝不落盘 |
| 输出抽象 | `Output` enum (Text/Json/Table) + `Renderer` | 一处切换，全局生效 |
| 模块抽象 | `Executor` trait + `Box<dyn Executor>` | 主程序与模块解耦 |
| 模块范围 | 仅外部集成类（mail/cal/rss/note/todo）+ config | fs/net/sys 封装通用能力，代理可用 shell/curl/fd/rg 直接完成，已移除 |
| 错误处理（Notion） | 复用现有 `AgentError`（`Auth`/`Network`/`Config`/`Other`） | 设计文档建议新增变体，但与 note 映射重复、会分裂错误分类 |
| 非测试代码 | 禁止 `unwrap()`/`expect()` | 安全红线；`NotionClient::new` 改为返回 `Result` |
| 配置回写 | `toml` crate 的 `toml::Value` 局部编辑 | 不引入 `toml_edit`，零新增依赖 |
| HTTP 栈 | reqwest（rustls-tls）+ 共享 `notion_client` | note/todo/rss 复用，未引入新 HTTP 依赖 |

---

## 多账户 config.toml 设计草案

```toml
[default_account]
mail = "work"
calendar = "personal"

[[mail.accounts]]
name = "work"
imap_host = "imap.example.com"
imap_port = 993
smtp_host = "smtp.example.com"
smtp_port = 465
username = "me@example.com"
# password 不存这里，走 keyring service="everyday/mail/work"

[[mail.accounts]]
name = "personal"
imap_host = "imap.gmail.com"
...

[[calendar.accounts]]
name = "personal"
caldav_url = "https://caldav.example.com/user"
username = "me"
```

---

## Errors Encountered（仍相关项）

| Error | Resolution |
|-------|------------|
| lettre `imap-pool` feature 不存在 | 改为 `pool` + `tokio1-rustls-tls` + `builder` |
| `format!("{s:<0$}", s, w)` 位置参数错位 | 改用 `pad()` 自由函数手动补空格 |
| `toml::Value::is_boolean` 不存在 | 改为 `is_bool()` |
| clippy `needless_range_loop` | 用 `cells.iter().zip(widths.iter()).enumerate()` 替换 range 索引 |
| `mailparse` Envelope 字段是 `Cow<[u8]>` 非 `Cow<str>` | 用 `String::from_utf8_lossy` 转字符串 |
| async-imap 基于 `futures` AsyncRead，tokio-rustls 是 tokio 的 | `tokio-util` compat 桥接：`tls_stream.compat()` |
| `async_imap::types::Address` 路径不存在 | `Fetch::envelope()` 是方法（非字段），Address 来自 `imap_proto`，用类型推断避免命名 |
| `uid_search` 返回 `HashSet<u32>` 非 Stream | 直接 collect，不 try_collect |
| `mailparse::MailHeaderMap` 是 trait 不能作参数类型 | 改 `&mailparse::ParsedMail`，访问 `.headers` |
| `lettre` `ContentType::TEXT_PLAIN_UTF_8` 不存在 | 改 `ContentType::TEXT_PLAIN` |
| `config get/set` 不支持数组索引 | get_dotted/set_dotted 增加 array 分支，数字 seg 访问数组元素 |
| `http::Uri` 方法是 `host()` 非 `host_str()`（与 url::Url 混淆） | 改用 `base.host()` |
| `base` 被 `host` 借用后 move 到 `WebDavClient::new` | `host` 转 owned `String`（`.to_string()`）解除借用 |
| QQ CalDAV 不支持 current-user-principal（PROPFIND 404） | `find_current_user_principal` 失败时降级用 `base_url` 作 calendar home set |
| libdav `bootstrap_via_service_discovery` fallback DNS SRV（QQ 无 SRV，os error 10054） | `CalDavClient::new(webdav)` 跳过 bootstrap，手动 `find_context_path` 只做 well-known 重定向 |

---

## Phase 状态汇总
- Phase 1: complete
- Phase 2: complete
- Phase 3: complete
- Phase 4: complete
- Phase 5: complete
- Phase 6: complete
- Phase 7: complete
