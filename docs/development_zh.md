# 开发

项目结构、技术栈、构建、架构与实现状态。本文件是根 README「项目结构」「开发」
与「实现状态」章节的完整版。

- [English](development.md) · [中文](development_zh.md)

---

## 项目结构

```
everyday/
├── src/
│   ├── main.rs          # 入口：解析 → 分发 → 渲染
│   ├── cli.rs           # clap 命令定义
│   ├── config.rs        # 配置加载与多账户管理
│   ├── error.rs         # 统一错误类型 AgentError
│   ├── output.rs        # Output（Text/Json/Records 渲染）
│   └── modules/
│       ├── mod.rs       # Executor trait + ModuleRegistry
│       ├── email.rs     # 邮件（IMAP/SMTP）
│       ├── calendar.rs  # 日历（CalDAV）
│       ├── rss.rs       # RSS/Atom
│       ├── note.rs      # 笔记与知识库（本地 SQLite）
│       ├── todo.rs      # 待办任务（本地 SQLite）
│       └── bookmark.rs  # 书签（本地 SQLite）
├── skills/
│   ├── README.md              # 面向 Agent 用户的精简项目介绍
│   └── everyday-cli/
│       ├── SKILL.md           # Agent Skill 入口（遵循 agentskills.io 规范）
│       └── references/
│           ├── COMMANDS.md    # 完整命令参考（按需加载）
│           ├── TASKS.md       # 常见任务配方（按需加载）
│           └── MEMORY.md      # memory 语义与命名约定
├── Cargo.toml
├── config.example.toml
└── agents.md            # AI Agent 协作规范
```

## 开发

### 技术栈

- **语言**：Rust (edition 2024)
- **异步运行时**：tokio
- **CLI 解析**：clap (derive)
- **序列化**：serde + serde_json + toml
- **邮件**：async-imap (IMAP) + lettre (SMTP) + mailparse
- **凭证**：keyring（系统密钥环）
- **TLS**：rustls + webpki-roots

### 构建

```bash
cargo build
cargo clippy -- -D warnings
cargo test
```

### 架构

核心设计基于 `Executor` trait，主程序通过 trait object 调度，模块间解耦：

```rust
#[async_trait]
pub trait Executor: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn actions(&self) -> Vec<ActionDoc>;
    async fn execute(&self, action: &str, args: &[String], ctx: &RequestContext) -> Result<Output>;
}
```

新增模块只需：新建文件 + 实现 trait + 注册一行。详见 [`agents.md`](../agents.md)。

## 实现状态

| 模块 | 状态 | 说明 |
|------|------|------|
| `config` | ✅ 完整可用 | path / list / get / set / init |
| `mail` | ✅ 完整可用 | IMAP 收件 + SMTP 发件 + keyring 凭证 |
| `cal` | ✅ 完整可用 | CalDAV calendars / list / add / delete |
| `rss` | ✅ 完整可用 | follow / list / unfollow / digest / fetch |
| `note` | ✅ 完整可用 | search / list / create / read / append / update（本地 SQLite） |
| `todo` | ✅ 完整可用 | list / add / start / complete（本地 SQLite） |
| `bookmark` | ✅ 完整可用 | list / add（本地 SQLite） |
| `auth` | ✅ 完整可用（v0.8.0 新增） | login / logout / verify / list — 全模块统一的凭证生命周期管理 |
| `timeline` | ✅ 完整可用 | 统一事件流：today / yesterday / week / month / sync |
| `search` | ✅ 完整可用（v0.7.0 新增） | 跨模块统一搜索：query |
| `memory` | ✅ 完整可用（v0.10.0 新增） | append-only `(subject, predicate, object)` 三元组笔记本 + graph + Searchable |
| `health` | ✅ 完整可用（v0.11.0 新增） | 根级运维命令：所有模块本地健康检查，退出码 0/1 |
| `sync` | ✅ 完整可用（v0.13.0 新增） | WebDAV 双向文件同步：4 个用户 DB + config.toml，LWW 冲突双副本，`--push-only` / `--pull-only` / `--force`，opt-in auto_sync |

