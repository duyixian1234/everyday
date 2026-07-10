# Everyday 开发计划

**项目：** Everyday — The Rust-powered hands for your AI Agent
**范围：** 以 `agents.md`「范围与定位」节为权威说明（原 PRD.md 已移除）
**启动时间：** 2026-07-08
**目标：** 按 PRD V2.0 构建 CLI 工具集，先完成基础架构（agents.md / config / Executor trait / Output），再逐步填充模块。

---

## 总体目标

打造高性能、内存安全的本地 CLI 工具集，作为 AI Agent 的"数字双手"。统一命令结构 `everyday <module> <action> [options]`，支持 Text / JSON 双输出模式，JSON 为 AI 交互主模式。

---

## 阶段规划

### Phase 1: 项目地基与文档 [complete]
- [x] 更新 `Cargo.toml`：包名改为 `everyday`，加入核心依赖（tokio, clap, serde, serde_json, toml, dirs, anyhow/thiserror, keyring, sysinfo, reqwest, scraper, ignore, walkdir, async-imap, lettre, caldav, feed-rs, arboard, notify, chrono, tabled）
- [x] 创建 `agents.md`：定义 AI Agent 协作规范（项目结构、命令、编码约定、提交规范、测试要求）
- [x] 按 PRD 建立 `src/` 目录骨架（cli.rs, config.rs, error.rs, output.rs, modules/mod.rs + 子模块）
- [x] 验证 `cargo build` 能通过空骨架

### Phase 2: 配置系统（多账户） [complete]
- [x] 设计 `config.toml` 多账户结构：每个模块支持命名账户列表，标记 `default`
- [x] 实现 `config.rs`：加载、合并默认值、路径解析（`~/.config/everyday/config.toml`）
- [x] 实现 `everyday config set/get/list/path/init` 子命令
- [x] 凭证安全：密码走 `keyring` 约定（service=`everyday/<module>/<account>`），config.toml 只存账户元数据
- [x] 提供 `config.example.toml` 示例

### Phase 3: 核心抽象 [complete]
- [x] `error.rs`：统一 `AgentError` 枚举 + JSON 错误格式 `{"error":"...","message":"..."}`
- [x] `output.rs`：`Output` 结构体，支持 `Text` / `Json` / `Records` 三种渲染，`--json` 切换
- [x] `modules/mod.rs`：定义 `Executor` trait（`async fn execute(&self, action, args) -> Result<Output>`）
- [x] 模块注册表：`ModuleRegistry` 按 name 查找 trait object

### Phase 4: CLI 框架 [complete]
- [x] `cli.rs`：clap derive，扁平 `everyday <module> <action> [args]` + `--json`/`--account` 全局 flag
- [x] `main.rs`：解析 → 加载配置 → 查找模块 → 执行 → 渲染输出 → 退出码
- [x] `config` 子命令特殊处理（读写配置文件）

### Phase 5: 模块骨架 [complete]
- [x] `modules/email.rs`、`calendar.rs`、`rss.rs`、`system.rs`、`network.rs`、`fs.rs`
- [x] 每个模块实现 `Executor`，未实现动作返回 `NotImplemented`，未知动作返回 `UnknownAction`
- [x] 在 `modules/mod.rs` 注册所有模块
- [x] `system` 模块 `status` 已可工作（sysinfo，作为参考实现）

### Phase 6: 参考模块实现 [pending]
- [x] `email` 模块（IMAP list/read/search + SMTP send + keyring login）[2026-07-08 完成]
- [x] `calendar` 模块（CalDAV：login/calendars/list/add/delete，libdav+icalendar）[2026-07-09 完成]
- [x] `rss` 模块（feed-rs）[2026-07-09 完成]
- [x] `note` 模块（Notion 笔记/知识库：login/search/create/read/append/update/list）[2026-07-10 完成]
- [x] `notion-client` 共享 SDK + `todo` 模块（Notion 待办：login/init-db/list/add/start/complete）[2026-07-10 完成]

> **范围变更（2026-07-10）**：经设计评审，移除 `fs`、`net` 与 `sys` 模块。理由：`fs`/`net` 封装的是代理可用 shell / `curl` / `fd` / `rg` 直接完成的通用能力，无明显差异化价值；`sys`（系统资源监控）亦属代理可经系统工具直接获取的信息，与 everyday「外部集成接口」定位不符。最终保留 `mail` / `cal` / `rss` 三个外部集成模块（+ `config` 配置管理）。详见 `findings.md`。

### Phase 7: 构建、测试、文档 [pending]
- [ ] 全模块 `cargo build` 全绿，`cargo clippy -- -D warnings` 无警告
- [ ] 单元测试覆盖各模块核心动作
- [ ] 集成测试：各模块 `--json` 输出合法 JSON
- [ ] README 使用示例 + 各模块命令清单

---

## 关键设计决策

| 决策点 | 选择 | 理由 |
|---|---|---|
| 包名 | `everyday` | PRD 指定 |
| 异步运行时 | `tokio` | PRD 推荐，生态成熟 |
| CLI 解析 | `clap` (derive) | PRD 推荐，类型安全 |
| 错误处理 | `thiserror` + `Result<T, AgentError>` | 统一错误类型，易序列化 |
| 配置格式 | TOML | PRD 指定，人类可读 |
| 凭证存储 | `keyring` (系统密钥环) | PRD 安全要求，禁明文 |
| 输出抽象 | `Output` enum (Text/Json) + `Renderer` | 一处切换，全局生效 |
| 模块抽象 | `Executor` trait + `Box<dyn Executor>` | 主程序与模块解耦 |
| 模块范围 | 仅保留外部集成类（mail/cal/rss/note/todo）+ config 配置管理 | fs/net/sys 封装通用能力，与定位不符，已移除 |
| 错误处理 | 复用现有 `AgentError`（`Auth`/`Network`/`Config`/`Other`） | 设计文档建议新增 `NotionApiError` 等变体，但与既有 note 映射重复、会分裂错误分类，故不新增 |
| 非测试代码 | 禁止 `unwrap()`/`expect()` | 设计文档 `NotionClient::new` 用 unwrap，已改为返回 `Result` |
| 配置回写 | `toml` crate 的 `toml::Value` 局部编辑 | 设计文档建议 `toml_edit`；项目已统一用 `toml`，零新增依赖 |
| 凭证存储 | `keyring`（service=`everyday/<module>/<account>`） | 设计文档与本项目一致；Token 绝不落盘 |

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

## Errors Encountered
| Error | Attempt | Resolution |
|-------|---------|------------|
| lettre `imap-pool` feature 不存在 | 1 | 改为 `pool` + `tokio1-rustls-tls` + `builder` |
| `format!("{s:<0$}", s, w)` 位置参数错位 | 1 | 改用 `pad()` 自由函数手动补空格 |
| sysinfo 0.30 `System::global_cpu_usage()` / `disks()` 不存在 | 1 | CPU 改用 `sys.cpus()` 平均；磁盘改用 `sysinfo::Disks::new_with_refreshed_list()` |
| `toml::Value::is_boolean` 不存在 | 1 | 改为 `is_bool()` |
| clippy `needless_range_loop` | 1 | 用 `cells.iter().zip(widths.iter()).enumerate()` 替换 range 索引 |
| `mailparse` Envelope 字段是 `Cow<[u8]>` 非 `Cow<str>` | 1 | 用 `String::from_utf8_lossy` 转字符串 |
| async-imap 基于 `futures` AsyncRead，tokio-rustls 是 tokio 的 | 1 | `tokio-util` compat 桥接：`tls_stream.compat()` |
| `async_imap::types::Address` 路径不存在 | 1 | `Fetch::envelope()` 是方法（非字段），Address 来自 `imap_proto`，用类型推断避免命名 |
| `uid_search` 返回 `HashSet<u32>` 非 Stream | 1 | 直接 collect，不 try_collect |
| `mailparse::MailHeaderMap` 是 trait 不能作参数类型 | 1 | 改 `&mailparse::ParsedMail`，访问 `.headers` |
| `lettre` `ContentType::TEXT_PLAIN_UTF_8` 不存在 | 1 | 改 `ContentType::TEXT_PLAIN` |
| `config get/set` 不支持数组索引 | 1 | get_dotted/set_dotted 增加 array 分支，数字 seg 访问数组元素 |
| `http::Uri` 方法是 `host()` 非 `host_str()`（与 url::Url 混淆） | 1 | 改用 `base.host()` |
| `base` 被 `host` 借用后 move 到 `WebDavClient::new` | 1 | `host` 转 owned `String`（`.to_string()`）解除借用 |
| QQ CalDAV 不支持 current-user-principal（PROPFIND 404） | 1 | `find_current_user_principal` 失败时降级用 `base_url` 作 calendar home set |
| libdav `bootstrap_via_service_discovery` fallback DNS SRV（QQ 无 SRV，os error 10054） | 1 | `CalDavClient::new(webdav)` 跳过 bootstrap，手动 `find_context_path` 只做 well-known 重定向 |

---

## Phase 状态汇总
- Phase 1: complete
- Phase 2: complete
- Phase 3: complete
- Phase 4: complete
- Phase 5: complete
- Phase 6: 进行中（email + calendar + rss + note + notion-client/todo 完成；fs/net/sys 已移除）
- Phase 7: pending
