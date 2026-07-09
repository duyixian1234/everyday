# Everyday CLI

> The Rust-powered hands for your AI Agent.

`everyday` 是一款高性能、内存安全的本地 CLI 工具集，用 Rust 编写。它作为 AI Agent 的"数字双手"，统一命令结构，覆盖邮件、日历、系统监控、文件操作、网络抓取等场景，支持 Text / JSON 双输出模式。

## 特性

- **统一命令结构**：`everyday <module> <action> [options]`，学习成本低
- **双输出模式**：默认 Text（人类可读表格），`--json` 切换为纯净 JSON（AI 交互主模式）
- **多账户支持**：每个模块支持多个命名账户，`--account` 灵活切换
- **凭证安全**：密码走系统密钥环（macOS Keychain / Windows Credential Manager / Linux Secret Service），绝不落盘
- **跨平台**：Windows / macOS / Linux
- **高性能**：冷启动 < 100ms，异步运行时（tokio），内存安全

## 安装

### 从源码构建

```bash
git clone <repo-url>
cd everyday
cargo build --release
```

编译产物位于 `target/release/everyday`，将其加入 `PATH` 即可。

### 验证安装

```bash
everyday --version
everyday config path
```

## 快速开始

### 1. 初始化配置

```bash
# 生成示例配置文件
everyday config init

# 查看配置路径
everyday config path
# → ~/.config/everyday/config.toml
```

### 2. 配置邮件账户

编辑 `~/.config/everyday/config.toml`：

```toml
[default_account]
mail = "work"

[[mail.accounts]]
name = "work"
imap_host = "imap.example.com"
imap_port = 993
smtp_host = "smtp.example.com"
smtp_port = 587
username = "me@example.com"
tls = true
```

或用命令行逐项设置：

```bash
everyday config set default_account.mail work
everyday config set mail.accounts.0.name work
everyday config set mail.accounts.0.imap_host imap.example.com
everyday config set mail.accounts.0.smtp_host smtp.example.com
everyday config set mail.accounts.0.username me@example.com
```

### 3. 存储密码

```bash
everyday mail login --account work
# 提示输入密码，存入系统密钥环（不落盘）
```

### 4. 开始使用

```bash
# 列出未读邮件
everyday mail list --unread

# JSON 模式（AI 友好）
everyday mail list --unread --limit 10 --json

# 查看系统状态
everyday sys status
```

## 命令参考

### 全局选项

| 选项 | 说明 |
|------|------|
| `--json` | 输出纯净 JSON，适合程序化解析 |
| `--account <NAME>` | 覆盖模块的默认账户 |
| `--version` | 显示版本号 |
| `--help` | 显示帮助 |

### config — 配置管理

管理 `~/.config/everyday/config.toml` 配置文件。

| 命令 | 说明 | 用法 |
|------|------|------|
| `path` | 显示配置文件路径 | `everyday config path` |
| `list` | 列出全部配置 | `everyday config list [--json]` |
| `get` | 读取配置项（支持点分路径与数组索引） | `everyday config get <dotted.path>` |
| `set` | 设置配置项（自动推断类型） | `everyday config set <dotted.path> <value>` |
| `init` | 创建示例配置 | `everyday config init` |

**点分路径示例**：
```bash
everyday config get mail.accounts.0.name        # → work
everyday config get default_account.mail         # → work
everyday config set mail.accounts.0.imap_port 993
everyday config set default_account.mail personal
```

### mail — 邮件管理

基于 IMAP（收件）和 SMTP（发件）协议，凭证走系统密钥环。

| 命令 | 说明 | 用法 |
|------|------|------|
| `login` | 交互式存储密码到密钥环 | `everyday mail login [--account NAME]` |
| `folders` | 列出所有邮箱文件夹 | `everyday mail folders [--account NAME]` |
| `list` | 列出邮件摘要 | `everyday mail list [--unread] [--limit N] [--folder NAME] [--no-recursive]` |
| `read` | 读取单封邮件（默认递归查找） | `everyday mail read <uid> [--folder NAME] [--no-recursive]` |
| `search` | 搜索邮件 | `everyday mail search --query Q [--limit N] [--folder NAME]` |
| `send` | 发送邮件 | `everyday mail send --to ADDR --subject S --body TEXT [--cc ADDR]` |

**选项说明**：

| 选项 | 适用命令 | 说明 |
|------|----------|------|
| `--account NAME` | 全部 | 指定账户 |
| `--unread` | `list` | 仅未读 |
| `--limit N` | `list` / `search` | 限制条数，默认 20 |
| `--folder NAME` | `list` / `read` / `search` | 指定文件夹（支持中文名），默认递归全部 |
| `--no-recursive` | `list` / `read` / `search` | 仅查 INBOX |
| `--to ADDR` | `send` | 收件人（必填） |
| `--subject S` | `send` | 主题（必填） |
| `--body TEXT` | `send` | 正文（必填） |
| `--cc ADDR` | `send` | 抄送 |

**递归搜索**：`list` / `search` / `read` 默认遍历所有文件夹。`list` / `search` 跨文件夹按邮件日期降序合并；`read` 找到首个命中 UID 的邮件即返回（IMAP UID 仅文件夹内唯一，跨文件夹不唯一，故需递归查找）。

### sys — 系统监控

| 命令 | 说明 | 状态 | 用法 |
|------|------|------|------|
| `status` | CPU / 内存 / 磁盘使用率 | ✅ 可用 | `everyday sys status [--json]` |
| `watch` | 监听文件系统变化 | 待实现 | `everyday sys watch <path>` |
| `clip` | 读写系统剪贴板 | 待实现 | `everyday sys clip [get\|set VALUE]` |

### fs — 文件操作

| 命令 | 说明 | 状态 | 用法 |
|------|------|------|------|
| `search` | 按文件名或内容搜索 | 待实现 | `everyday fs search [--content PATTERN] [--path P] [NAME-GLOB]` |
| `tree` | 目录树 | 待实现 | `everyday fs tree [--path P] [--max-depth N]` |
| `read-json` | 读取并美化 JSON/TOML | 待实现 | `everyday fs read-json <path>` |

### net — 网络工具

| 命令 | 说明 | 状态 | 用法 |
|------|------|------|------|
| `fetch` | 抓取网页并清洗为 Markdown | 待实现 | `everyday net fetch <url>` |
| `request` | 通用 HTTP 请求 | 待实现 | `everyday net request --method POST --url URL [--body '...']` |

### cal — 日历管理（CalDAV）

| 命令 | 说明 | 状态 | 用法 |
|------|------|------|------|
| `list` | 列出日程 | 待实现 | `everyday cal list [--today\|--date YYYY-MM-DD]` |
| `add` | 添加日程 | 待实现 | `everyday cal add --title T --start ISO --end ISO` |
| `delete` | 删除日程 | 待实现 | `everyday cal delete --id ID` |

### rss — RSS/Atom 订阅

| 命令 | 说明 | 状态 | 用法 |
|------|------|------|------|
| `follow` | 添加订阅源 | 待实现 | `everyday rss follow --name N --url URL [--category C]` |
| `list` | 列出订阅源 | 待实现 | `everyday rss list` |
| `digest` | 聚合近期内容 | 待实现 | `everyday rss digest [--limit N]` |

## 输出模式

### Text 模式（默认）

适合终端直接查看，表格自动对齐：

```
$ everyday mail list --unread --limit 3
uid    folder  date                          from              subject
-----------------------------------------------------------------------------
12345  INBOX   Wed, 8 Jul 2026 08:29 +0000  sender@x.com      Hello
12344  INBOX   Wed, 8 Jul 2026 07:15 +0000  boss@x.com        Weekly Report
12343  Drafts  Wed, 8 Jul 2026 06:00 +0000  me@x.com          Draft
```

### JSON 模式（`--json`）

输出纯净 JSON，无多余空白，适合程序化解析：

```bash
$ everyday mail list --unread --limit 2 --json
[{"uid":"12345","folder":"INBOX","date":"Wed, 8 Jul 2026 08:29:31 +0000","from":"sender@x.com","subject":"Hello"},{"uid":"12344","folder":"INBOX","date":"Wed, 8 Jul 2026 07:15:00 +0000","from":"boss@x.com","subject":"Weekly Report"}]
```

### 错误输出

JSON 模式下错误格式：

```json
{"error": "AccountNotFound", "message": "mail account 'work'"}
```

退出码：成功 `0`，失败 `1`。

## 配置

配置文件路径：`~/.config/everyday/config.toml`

```toml
[default_account]
mail = "work"
calendar = "personal"

[[mail.accounts]]
name = "work"
imap_host = "imap.example.com"
imap_port = 993          # 默认 993
smtp_host = "smtp.example.com"
smtp_port = 587          # 默认 587
username = "me@example.com"
tls = true               # 默认 true

[[mail.accounts]]
name = "personal"
imap_host = "imap.gmail.com"
imap_port = 993
smtp_host = "smtp.gmail.com"
smtp_port = 587
username = "me@gmail.com"
tls = true

[[calendar.accounts]]
name = "personal"
caldav_url = "https://caldav.example.com/me"
username = "me"

[[rss.feeds]]
name = "hackernews"
url = "https://hnrss.org/frontpage"
category = "tech"
```

### 凭证安全

密码**绝不**存储在配置文件中，而是通过系统密钥环管理：

- **keyring 服务名约定**：`everyday/<module>/<account>`（如 `everyday/mail/work`）
- **存储密码**：`everyday mail login --account work`（交互式输入，存入密钥环）
- **读取密码**：其他 mail 命令自动从密钥环读取，无需手动指定

### 多账户

每个模块支持多个命名账户：

- 配置文件中通过 `[[mail.accounts]]` 等数组定义
- `[default_account]` 指定各模块的默认账户名
- `--account NAME` 覆盖默认账户

## 使用示例

### 邮件

```bash
# 列出所有文件夹
everyday mail folders

# 查看最近 10 封未读邮件（JSON）
everyday mail list --unread --limit 10 --json

# 在指定文件夹中查找邮件
everyday mail search --query "invoice" --folder INBOX --json

# 读取某封邮件
everyday mail read 12345 --json

# 发送邮件
everyday mail send \
  --to recipient@example.com \
  --subject "周报" \
  --body "本周工作总结..." \
  --cc manager@example.com

# 切换账户
everyday mail list --account personal --json
```

### 系统

```bash
# 查看系统资源
everyday sys status

# JSON 格式（便于监控脚本）
everyday sys status --json
```

### 配置

```bash
# 初始化
everyday config init

# 查看配置
everyday config list

# 读取某项
everyday config get mail.accounts.0.username

# 修改某项
everyday config set mail.accounts.0.smtp_port 465

# 验证
everyday config get mail.accounts.0.smtp_port
```

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
│       ├── system.rs    # 系统监控
│       ├── network.rs   # 网页抓取/HTTP
│       └── fs.rs        # 文件搜索/目录树
├── skills/
│   ├── README.md              # 面向 Agent 用户的精简项目介绍
│   └── everyday-cli/
│       ├── SKILL.md           # Agent Skill 入口（遵循 agentskills.io 规范）
│       └── references/
│           └── COMMANDS.md    # 完整命令参考（按需加载）
├── Cargo.toml
├── config.example.toml
├── PRD.md
└── agents.md            # AI Agent 协作规范
```

## 开发

### 技术栈

- **语言**：Rust (edition 2024)
- **异步运行时**：tokio
- **CLI 解析**：clap (derive)
- **序列化**：serde + serde_json + toml
- **邮件**：async-imap (IMAP) + lettre (SMTP) + mailparse
- **系统信息**：sysinfo
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
    async fn execute(&self, action: &str, args: &[String]) -> Result<Output>;
}
```

新增模块只需：新建文件 + 实现 trait + 注册一行。详见 [`agents.md`](../agents.md)。

## 实现状态

| 模块 | 状态 | 说明 |
|------|------|------|
| `config` | ✅ 完整可用 | path / list / get / set / init |
| `mail` | ✅ 完整可用 | IMAP 收件 + SMTP 发件 + keyring 凭证 |
| `sys` | ✅ 部分可用 | `status` 可用；`watch` / `clip` 待实现 |
| `fs` | 🚧 待实现 | search / tree / read-json |
| `net` | 🚧 待实现 | fetch / request |
| `cal` | 🚧 待实现 | CalDAV list / add / delete |
| `rss` | 🚧 待实现 | follow / list / digest |

## 许可证

MIT
