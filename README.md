# Everyday CLI

> The Rust-powered hands for your AI Agent.

`everyday` 是一款高性能、内存安全的本地 CLI 工具集，用 Rust 编写。它作为 AI Agent 的"数字双手"，统一命令结构，覆盖邮件、日历、RSS 订阅、笔记（默认本地 SQLite / 可选 Notion）、待办（默认本地 SQLite / 可选 Notion）等外部集成场景，支持 Text / JSON 双输出模式。

## 特性

- **统一命令结构**：`everyday <module> <action> [options]`，学习成本低
- **双输出模式**：默认 Text（人类可读表格），`--json` 切换为纯净 JSON（AI 交互主模式）
- **多账户支持**：每个模块支持多个命名账户，`--account` 灵活切换
- **凭证安全**：密码走系统密钥环（macOS Keychain / Windows Credential Manager / Linux Secret Service），绝不落盘
- **跨平台**：Windows / macOS / Linux
- **高性能**：冷启动 < 100ms，异步运行时（tokio），内存安全

## 安装

### 下载预编译二进制（推荐）

从 [GitHub Releases](https://github.com/duyixian1234/everyday/releases) 下载对应平台的压缩包，解压后将 `everyday` 加入 `PATH` 即可。每个 Release 附带各平台（含 macOS x86_64 与 Apple Silicon / aarch64）的资产：

| 平台 | 资产文件 | 解压 / 安装 |
|------|----------|-------------|
| Linux (x86_64) | `everyday-x86_64-unknown-linux-gnu.tar.gz` | `tar xzf <file> && sudo mv everyday /usr/local/bin/` |
| macOS (x86_64) | `everyday-x86_64-apple-darwin.tar.gz` | `tar xzf <file> && sudo mv everyday /usr/local/bin/` |
| macOS (Apple Silicon / aarch64) | `everyday-aarch64-apple-darwin.tar.gz` | `tar xzf <file> && sudo mv everyday /usr/local/bin/` |
| Windows (x86_64) | `everyday-x86_64-pc-windows-msvc.zip` | 解压后将 `everyday.exe` 放入 `PATH` 目录 |

macOS / Linux 一行安装（自动取 latest）：

```bash
# Linux
curl -L https://github.com/duyixian1234/everyday/releases/latest/download/everyday-x86_64-unknown-linux-gnu.tar.gz | tar xz && sudo mv everyday /usr/local/bin/

# macOS (Intel)
curl -L https://github.com/duyixian1234/everyday/releases/latest/download/everyday-x86_64-apple-darwin.tar.gz | tar xz && sudo mv everyday /usr/local/bin/

# macOS (Apple Silicon)
curl -L https://github.com/duyixian1234/everyday/releases/latest/download/everyday-aarch64-apple-darwin.tar.gz | tar xz && sudo mv everyday /usr/local/bin/
```

> 二进制由 CI 在每次打 `v*` tag 时自动构建并发布（见 `.github/workflows/release.yml`），覆盖 Linux / macOS（x86_64 与 aarch64）/ Windows 三平台四架构。

### 从源码构建

```bash
git clone https://github.com/duyixian1234/everyday.git
cd everyday
cargo build --release
```

编译产物位于 `target/release/everyday`，将其加入 `PATH` 即可。

### 通过 cargo 安装

```bash
cargo install --git https://github.com/duyixian1234/everyday.git
```

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

### cal — 日历管理（CalDAV）

| 命令 | 说明 | 状态 | 用法 |
|------|------|------|------|
| `list` | 列出日程 | ✅ 可用 | `everyday cal list [--today\|--date YYYY-MM-DD]` |
| `add` | 添加日程 | ✅ 可用 | `everyday cal add --title T --start ISO --end ISO` |
| `delete` | 删除日程 | ✅ 可用 | `everyday cal delete --id ID` |

### rss — RSS/Atom 订阅

| 命令 | 说明 | 状态 | 用法 |
|------|------|------|------|
| `follow` | 添加订阅源 | ✅ 可用 | `everyday rss follow --name N --url URL [--category C]` |
| `list` | 列出订阅源 | ✅ 可用 | `everyday rss list` |
| `digest` | 聚合近期内容 | ✅ 可用 | `everyday rss digest [--limit N]` |

### note — 笔记与知识库（默认本地 SQLite / 可选 Notion）

**默认使用本地 SQLite provider（`provider = "local"`，别名 `sqlite`）**：无需凭证、无需联网，数据存于 `~/.config/everyday/note-<account>.db`，开箱即用。也可设 `provider = "notion"` 改用 Notion API，屏蔽繁琐的 Block 嵌套，向 Agent 暴露**纯文本 / Markdown 追加**与**简化属性操作**两个高层能力（Notion Integration Token 仅存系统密钥环，绝不落盘）。命令用法在两种 provider 下一致。

| 命令 | 说明 | 用法 |
|------|------|------|
| `login` | 交互式存储 Notion Token 到密钥环（仅 `notion` provider 需要） | `everyday note login [--account NAME]` |
| `search` | 按标题搜索页面 / 数据库 | `everyday note search --query Q [--limit N]` |
| `list` | 列出指定数据库下的页面 | `everyday note list [--db ID] [--limit N]` |
| `create` | 在数据库中新建页面（记录） | `everyday note create --title T [--db ID] [--prop K:V ...]` |
| `read` | 读取页面正文，聚合成 Markdown | `everyday note read <page_id>` |
| `append` | 向页面末尾追加文本区块 | `everyday note append [page_id] --text TEXT` |
| `update` | 修改页面属性（Meta 信息） | `everyday note update <page_id> --prop K:V ...` |

**选项说明**：

| 选项 | 适用命令 | 说明 |
|------|----------|------|
| `--account NAME` | 全部 | 指定账户 |
| `--query Q` | `search` | 关键词搜索（页面 / 数据库标题） |
| `--db ID` | `create` / `list` | 目标数据库 ID；未指定则读取配置 `default_database_id` |
| `--prop K:V` | `create` / `update` | 简化属性设置，可多次指定；按数据库 schema 精确编码（标题 / 文本 / 数字 / Checkbox / Select 等），值可含冒号 |
| `--text TEXT` | `append` | 要追加的文本；不带此参数时从管道 `stdin` 读取（仅非终端模式） |
| `--limit N` | `search` / `list` | 限制条数（`search` 默认 10，`list` 默认 50，上限 100；`--limit 0` 表示不限制） |

> **本地 provider（默认）**：无需任何前置步骤，直接 `everyday note create` / `append` 即可，数据库文件自动创建。
> **Notion provider**：在 Notion 创建 integration 拿到 `ntn_...` token → `everyday note login` 存入密钥环 → 在 config 把该账户设为 `provider = "notion"` 并填好 `default_database_id` / `default_page_id` → 在 Notion 把目标页面 / 数据库**分享给该 integration**。

### todo — 待办任务（默认本地 SQLite / 可选 Notion）

**默认使用本地 SQLite provider（`provider = "local"`，别名 `sqlite`）**：无需凭证、无需联网，任务存于 `~/.config/everyday/todo-<account>.db`，各命令自动建表，开箱即用。也可设 `provider = "notion"` 改用 Notion 数据库：底层 HTTP / Token 注入 / 429 限流重试由共享 `notion-client` 统一处理，本模块把干净的领域模型 `TodoItem`（id / title / status / due / priority）与 Notion 原始属性做强类型映射（Token 仅存系统密钥环 `everyday/todo/<account>`，`database_id` 等非机密元数据可落盘 config）。命令用法在两种 provider 下一致。

| 命令 | 说明 | 用法 |
|------|------|------|
| `login` | 交互式存储 Notion Token 到密钥环（仅 `notion` provider 需要） | `everyday todo login [--account NAME]` |
| `init-db` | 建表：本地 provider 建 SQLite 表，Notion provider 创建任务数据库（需 `parent_page_id`）并回填 `database_id` | `everyday todo init-db [--account NAME] [--parent PAGE_ID]` |
| `list` | 列出未完成任务（按 Due 升序） | `everyday todo list [--db ID] [--all]` |
| `add` | 新增任务 | `everyday todo add --title T [--due DATE] [--priority P] [--db ID]` |
| `start` | 标记任务为 In Progress | `everyday todo start <page_id>` |
| `complete` | 标记任务为 Done | `everyday todo complete <page_id>` |

**选项说明**：

| 选项 | 适用命令 | 说明 |
|------|----------|------|
| `--account NAME` | 全部 | 指定账户 |
| `--parent PAGE_ID` | `init-db` | 创建数据库时的父级页面；未指定则读取配置 `parent_page_id` |
| `--db ID` | `list` / `add` | 目标数据库 ID；未指定则读取配置 `default_database_id`（`init-db` 后自动回填） |
| `--all` | `list` | 列出全部任务（含已完成的 Done） |
| `--title T` | `add` | 任务标题（必填） |
| `--due DATE` | `add` | 截止日期（ISO 8601，如 `2026-07-15`） |
| `--priority P` | `add` | 优先级（Select：P0 / P1 / P2） |

> **本地 provider（默认）**：无需任何前置步骤，直接 `everyday todo add` / `list` 即可，数据库文件与表自动创建。
> **Notion provider**：在 Notion 创建 integration 拿到 `ntn_...` token → `everyday todo login` 存入密钥环 → 在 config 把该账户设为 `provider = "notion"` 并填好 `parent_page_id` → `everyday todo init-db` 创建任务数据库并授权该 integration 访问父级页面。之后 `list` / `add` / `start` / `complete` 即可使用。

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
note = "personal"

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

# 笔记 / 待办默认本地 SQLite provider，开箱即用、无需凭证
[[note.accounts]]
name = "personal"
provider = "local"
# db_path = "/absolute/path/to/notes.db"   # 可选，缺省 ~/.config/everyday/note-personal.db

[[todo.accounts]]
name = "personal"
provider = "local"
# db_path = "/absolute/path/to/todos.db"   # 可选，缺省 ~/.config/everyday/todo-personal.db

# 如需 Notion：把对应账户改为 provider = "notion"，并按各模块「前置」说明配置
# [[note.accounts]]
# name = "notion"
# provider = "notion"
# default_database_id = "db_abc123..."   # 值以实际 Notion ID 为准
# default_page_id = "page_xyz789..."
# Notion Integration Token (ntn_...) 不在此处填写，由 `everyday note login` 存入密钥环
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

### 笔记（默认本地 SQLite）

```bash
# 本地 provider 无需登录；仅 provider = "notion" 时才需交互式存入 Token（仅密钥环，不落盘）
everyday note login

# 搜索页面 / 数据库（JSON）
everyday note search --query "工作" --json

# 列出某数据库下的页面（缺省取配置 default_database_id）
everyday note list --json
everyday note list --db "db_abc123" --limit 20

# 在数据库中新建一条记录，含多项属性
everyday note create \
  --title "Rust 异步运行时深入浅出" \
  --prop "类型:文章" \
  --prop "状态:未读" \
  --prop "URL:https://..."

# 读取页面正文（聚合成 Markdown）
everyday note read <page_id> --json

# 向默认速记页面追加一条闪念（也可不带 page_id 自动寻址 default_page_id）
everyday note append --text "### AI 自动捕获
在 12345 号邮件发现竞品链接：https://..."

# 管道方式追加
echo "批量捕获的内容" | everyday note append <page_id>

# 修改页面属性
everyday note update <page_id> --prop "状态:已读"
```

### 待办（默认本地 SQLite）

```bash
# 本地 provider 无需登录，直接 add / list 即可（表自动创建）；
# 仅 provider = "notion" 时才需以下一次性配置：存 Token、创建任务数据库
everyday todo login
everyday todo init-db --parent "<page_id>"     # 需在 Notion 把父页面授权给该 integration

# 列出未完成任务（按 Due 升序）
everyday todo list --json

# 全部任务（含已完成）
everyday todo list --all --json

# 新增任务
everyday todo add --title "写周报" --due 2026-07-15 --priority P1

# 状态切换（返回含 page id 与 url）
everyday todo start <page_id>
everyday todo complete <page_id>
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
│   ├── notion_client.rs # 底层共享 Notion API 客户端（HTTP/限流/反序列化）
│   └── modules/
│       ├── mod.rs       # Executor trait + ModuleRegistry
│       ├── email.rs     # 邮件（IMAP/SMTP）
│       ├── calendar.rs  # 日历（CalDAV）
│       ├── rss.rs       # RSS/Atom
│       ├── note.rs      # 笔记与知识库（Notion API）
│       └── todo.rs      # 待办任务（Notion，基于 notion_client）
├── skills/
│   ├── README.md              # 面向 Agent 用户的精简项目介绍
│   └── everyday-cli/
│       ├── SKILL.md           # Agent Skill 入口（遵循 agentskills.io 规范）
│       └── references/
│           └── COMMANDS.md    # 完整命令参考（按需加载）
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
    async fn execute(&self, action: &str, args: &[String]) -> Result<Output>;
}
```

新增模块只需：新建文件 + 实现 trait + 注册一行。详见 [`agents.md`](../agents.md)。

## 实现状态

| 模块 | 状态 | 说明 |
|------|------|------|
| `config` | ✅ 完整可用 | path / list / get / set / init |
| `mail` | ✅ 完整可用 | IMAP 收件 + SMTP 发件 + keyring 凭证 |
| `cal` | ✅ 完整可用 | CalDAV login / calendars / list / add / delete |
| `rss` | ✅ 完整可用 | follow / list / unfollow / digest / fetch |
| `note` | ✅ 完整可用 | login / search / list / create / read / append / update（默认本地 SQLite，可选 Notion API） |
| `todo` | ✅ 完整可用 | login / init-db / list / add / start / complete（默认本地 SQLite，可选 Notion API） |

## 许可证

MIT
