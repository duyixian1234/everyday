# 命令参考

每个 `everyday` 模块的完整命令表，以及全局选项、输出模式（text / JSON / 错误 / 日志）
与 CLI 行为。本文件是根 README「命令参考」与「输出模式」章节的完整版。

- [English](commands.md) · [中文](commands_zh.md)

---

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
| `folders` | 列出所有邮箱文件夹 | `everyday mail folders [--account NAME]` |
| `list` | 列出邮件摘要（本地缓存；过期自动 sync） | `everyday mail list [--unread] [--limit N] [--folder NAME] [--no-recursive] [--sync]` |
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
| `--sync` | `list` | 强制 IMAP sync 后再列出（忽略 staleness） |
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

### note — 笔记与知识库（本地 SQLite）

**使用本地 SQLite provider（`provider = "local"`，别名 `sqlite`）**：无需凭证、无需联网，数据存于 `~/.config/everyday/note-<account>.db`，开箱即用。

| 命令 | 说明 | 用法 |
|------|------|------|
| `search` | 按标题搜索页面 / 数据库 | `everyday note search --query Q [--limit N]` |
| `list` | 列出指定数据库下的页面 | `everyday note list [--limit N]` |
| `create` | 在数据库中新建页面（记录） | `everyday note create --title T [--prop K:V ...]` |
| `read` | 读取页面正文，聚合成 Markdown | `everyday note read <id>` |
| `append` | 向页面末尾追加文本区块 | `everyday note append [id] --text TEXT` |
| `update` | 修改页面属性（Meta 信息） | `everyday note update <id> --prop K:V ...` |

**选项说明**：

| 选项 | 适用命令 | 说明 |
|------|----------|------|
| `--account NAME` | 全部 | 指定账户 |
| `--query Q` | `search` | 关键词搜索（页面 / 数据库标题） |
| `--prop K:V` | `create` / `update` | 简化属性设置，可多次指定；按数据库 schema 精确编码（标题 / 文本 / 数字 / Checkbox / Select 等），值可含冒号 |
| `--text TEXT` | `append` | 要追加的文本；不带此参数时从管道 `stdin` 读取（仅非终端模式） |
| `--limit N` | `search` / `list` | 限制条数（`search` 默认 10，`list` 默认 50，上限 100；`--limit 0` 表示不限制） |

> **本地 provider（默认）**：无需任何前置步骤，直接 `everyday note create` / `append` 即可，数据库文件自动创建。

### todo — 待办任务（本地 SQLite）

**使用本地 SQLite provider（`provider = "local"`，别名 `sqlite`）**：无需凭证、无需联网，任务存于 `~/.config/everyday/todo-<account>.db`，各命令自动建表，开箱即用。

| 命令 | 说明 | 用法 |
|------|------|------|
| `list` | 列出未完成任务（按 Due 升序） | `everyday todo list [--all]` |
| `add` | 新增任务 | `everyday todo add --title T [--due DATE] [--priority P]` |
| `start` | 标记任务为 In Progress | `everyday todo start <id>` |
| `complete` | 标记任务为 Done | `everyday todo complete <id>` |

**选项说明**：

| 选项 | 适用命令 | 说明 |
|------|----------|------|
| `--account NAME` | 全部 | 指定账户 |
| `--all` | `list` | 列出全部任务（含已完成的 Done） |
| `--title T` | `add` | 任务标题（必填） |
| `--due DATE` | `add` | 截止日期（ISO 8601，如 `2026-07-15`） |
| `--priority P` | `add` | 优先级（Select：P0 / P1 / P2） |

> **本地 provider（默认）**：无需任何前置步骤，直接 `everyday todo add` / `list` 即可，数据库文件与表自动创建。

### bookmark — 书签（本地 SQLite）

**使用本地 SQLite provider（`provider = "local"`，别名 `sqlite`）**：无需凭证、无需联网，书签存于 `~/.config/everyday/bookmark-<account>.db`（主表 `bookmarks` + 关联表 `bookmark_tags`，支持按标签精确过滤），各命令自动建表，开箱即用。

| 命令 | 说明 | 用法 |
|------|------|------|
| `list` | 列出书签（`--tag` 按单个标签过滤） | `everyday bookmark list [--tag TAG]` |
| `add` | 新增书签 | `everyday bookmark add --url U --title T [--tags a,b]` |

**选项说明**：

| 选项 | 适用命令 | 说明 |
|------|----------|------|
| `--account NAME` | 全部 | 指定账户 |
| `--tag TAG` | `list` | 按单个标签过滤（精确匹配）；不指定则列出全部 |
| `--url U` | `add` | 书签 URL（必填） |
| `--title T` | `add` | 书签标题（必填） |
| `--tags a,b` | `add` | 逗号分隔的标签（可选，如 `rust,cli`） |

**标签解析**：`--tags "rust, cli , web"` 按逗号拆分、去空白、丢弃空项 → `["rust", "cli", "web"]`。

> **本地 provider（默认）**：无需任何前置步骤，直接 `everyday bookmark add` / `list` 即可，数据库文件与表自动创建。

### auth — 凭证生命周期（v0.8.0 新增）

全模块统一的凭证管理。各模块内部通过 `auth::get_credential` 读取已存凭证；你只需用这些命令在系统密钥环中管理凭证。密码凭证（mail/cal/webdav）使用 `--password`。若省略该 flag，则回退到交互式提示。密码绝不落盘。

**环境变量回退（可选，R020）**：系统无 keyring 后端（headless 服务器 / CI / 沙箱）时，可开启 `[auth] env_credentials = true`（或 `EVERYDAY_ENV_CREDENTIALS=1`）改从环境变量读取凭据，变量名 `EVERYDAY_<MODULE>_<ACCOUNT>_PASSWORD`（如 `EVERYDAY_MAIL_WORK_PASSWORD`）。读取链：keyring → env → 报错。两个开关均对全部模块生效（含 `mail list` / `cal` / `sync` 等 hot path）：配置字段在启动时同步为进程级开关，无需额外 export。env 来源的密码对每个子进程可见，仅在确无 keyring 的环境使用。

| 命令 | 说明 | 用法 |
|------|------|------|
| `login` | 将凭证存入系统密钥环（加 `--verify` 可同时校验）。`--module` 必填；`--account` 缺省为模块默认账户 | `everyday auth login --module mail --account work --password PWD` |
| `logout` | 从密钥环删除已存凭证；凭证来自环境变量时提示 `unset` | `everyday auth logout --module mail --account work` |
| `verify` | 读取已存凭证（keyring → env）并向服务端校验（不重新提示）；local/sqlite 或 rss 返回 `not_required` | `everyday auth verify --module note` |
| `list` | 列出已配置账户及其凭证状态（stored / env / missing / not_required） | `everyday auth list --module todo` |

WebDAV 设备同步请存**应用密码**（非登录密码）：`everyday auth login --module webdav --account personal`（keyring 键 `everyday/webdav/personal`）。

### timeline — 统一事件流（v0.5.0 新增）

将 **mail · cal · rss · note · todo · bookmark** 各 provider 的本地事件聚合到一个 append-only 事件流。每个 source 对应一个 `TimelineProvider` adapter，sync 在 source 间并行、在 source 内串行（对 rate-limit 友好）。存储为独立 SQLite：`~/.config/everyday/timeline.db`。

**为什么需要**：避免 Agent 分别轮询 7 个模块，单条 query 就拿到跨全部集成的时间有序事件流。

| 命令 | 说明 | 用法 |
|------|------|------|
| `today` / `yesterday` / `week` / `month` | 预设窗口查询（week 是 Mon–Sun，month 是日历月） | `everyday timeline today [--source S] [--account A] [--limit N] [--since 时长或日期]` |
| `sync` | 从所有（或 `--source` 过滤后的）provider 拉到 `timeline.db`；幂等,用水位控制 | `everyday timeline sync [--source mail,cal,todo] [--since 2026-01-01]` |

**常用 flag**：

| Flag | 适用 | 说明 |
|------|------|------|
| `--json` | 全部 | 切到 JSON 输出（Agent 推荐用） |
| `--source S[,S2]` | 全部 | 逗号分隔过滤,例如 `mail,cal` 或 `todo` |
| `--account A` | 全部 | 限定单个账户名（如 `personal`） |
| `--limit N` | query | 限制事件条数,默认 100 |
| `--since 时长或日期` | 全部 | 滑动窗口起点。`30m` / `2h` / `1d` / `7d` 相对 now,`YYYY-MM-DD` 当日 00:00 本地。to 一律是 `now()`。也支持 `--from` / `--to` 绝对窗口。 |
| `--sync` | query | 先 sync 再 query（原子） |

**示例**:

```bash
# 今天全部事件,JSON 输出
everyday timeline today --json | jq '.[].title'

# 仅 sync 邮件与日历,再查本周
everyday timeline sync --source mail,cal
everyday timeline week --json

# 最近 30 分钟内事件（sub-day 精度）
everyday timeline today --since 30m --json

everyday timeline today --source todo --json
```

**设计要点**：

- **Append-only**：事件以自然键 `(source, account, ref_id, event_type, timestamp)` 唯一（`INSERT OR IGNORE`），重跑 sync 安全。
- **UTC 存储 + 本地显示**：时间戳在 DB 内统一 UTC，渲染时按本地时区。
- **Cal 是窗口刷新**：除 mail / rss 是 append-only，cal 在每次 sync 时重写自己的窗口 `[last_sync, now+7d]`，这样取消的事件会真正消失。

完整设计依据见 `CONTEXT.md` 与 `adr/0001`–`0009`。

### search — 跨模块统一搜索（v0.7.0 新增）

一次查询跨所有模块。`everyday search` 并发扇出到每个已注册的 `Searchable` provider（note / todo / bookmark / rss / cal / mail / memory），合并命中、按时间排序后统一输出。空结果 exit 0；模块级失败以 `SearchWarning` 走 stderr（text）或结构化 `{"_warning": ...}` 行（`--json`），不中断整个查询。

| 命令 | 描述 | 用法 |
|------|------|------|
| `query` | 在所有可搜索模块上跑自由文本查询 | `everyday search query "<q>" [--module a,b,c] [--since 7d] [--limit N] [--json]` |

**模块范围**：`note` / `todo` / `bookmark`（本地 SQLite，GLOB 命中 title + content/url/tag），`rss`（本地条目缓存表 `~/.config/everyday/rss-items.db`，由 `rss digest` / `rss fetch` 同步写入），`cal`（全量拉取 + 内存 GLOB 命中 summary / location / start），`mail`（本地 envelope 缓存，[S007]，v0.9.0 起），`memory`（当前态视图上 subject/predicate/object 三字段 GLOB，[K003]，v0.10.0 起）。

**查询语义**：空白切 token、多词 **OR**、大小写不敏感 GLOB 子串（`lower(col) GLOB '*token*'`）。每模块硬上限 50；全局默认 20（可由 `--limit` 覆盖）。全局 `ts desc` 排序；各模块的主时间即 `ts`（note: updated_at，todo: updated_at，bookmark: created_at，rss: published，cal: event_start，mail: envelope date，memory: created_at）。

**示例**：

```bash
# 跨所有模块找 "rust" 相关的条目，JSON 输出
everyday search query "rust" --json

# 限定 note + todo，加 7 天下界
everyday search query "rust timeline" --module note,todo --since 7d

# 限制合并结果最多 5 条
everyday search query "release" --limit 5
```

### memory — Agent 结构化记忆笔记本（v0.10.0 新增）

Agent 自身的持久化、append-only 笔记本 —— 以 `(subject, predicate, object)` 三元组形式存放稳定事实，可附 `confidence` 与 `source`。三元组按版本演进：对同一 `(S, P, O)` 再次 `add` 会创建新行（旧行保留在 history 中）。软删除从当前态查询中隐藏该行，但 `history` 仍可见。存储为全局单例 SQLite 文件 `~/.config/everyday/memory.db`（无 `account` 列，不触及 `auth` 模块）。memory 作为 `Searchable` provider 自动参与 `everyday search`。

| 命令 | 描述 | 用法 |
|------|------|------|
| `add` | 追加三元组（重复 `(S,P,O)` 会创建新版本） | `everyday memory add <S> <P> <O> [--confidence N] [--source LABEL]` |
| `get` | 列出某 subject 当前态全部三元组 | `everyday memory get <SUBJECT>` |
| `relation` | 列出 `(subject, predicate)` 当前态匹配的对象 | `everyday memory relation <SUBJECT> <PREDICATE>` |
| `list` | 列出所有当前态三元组（默认上限 100） | `everyday memory list [--limit N]` |
| `delete` | 软删除某三元组的当前态行 | `everyday memory delete <S> <P> <O>` |
| `graph` | 从 subject 出发的前向 BFS（深度 1..=5，默认 2） | `everyday memory graph <SUBJECT> [--depth N] [--include-deleted]` |
| `history` | 查看某三元组的全部版本（含已删除行） | `everyday memory history <S> <P> <O>` |

**选项说明**：

| 选项 | 适用 | 描述 |
|------|------|------|
| `--confidence N` | `add` | 置信度，区间 `[0.0, 1.0]`，默认 `1.0` |
| `--source LABEL` | `add` | 来源标签自由文本（如 `explicit` / `inferred`） |
| `--limit N` | `list` | 行数上限，默认 100 |
| `--depth N` | `graph` | 递归深度，`1..=5`，默认 2 |
| `--include-deleted` | `graph` | 在遍历中包含软删除边 |

**Subject 命名约定**（程序不强制，约定见 `../skills/everyday-cli/references/MEMORY.md`）：

```
user                       # 表示用户本人
project-everyday           # 项目实体
tech:rust                  # 领域前缀：技术知识
team:backend:alice         # 层级：团队 > 子团队 > 人
```

**示例**：

```bash
# 记录用户偏好
everyday memory add user prefers rust --confidence 0.9 --source explicit --json

# 查询与用户相关的全部事实
everyday memory get user --json

# 多跳图遍历
everyday memory graph user --depth 2

# memory 自动接入 search
everyday search query "rust" --module memory --json
```

### health — 模块健康检查（v0.11.0 新增）

运行所有模块的 `health_check`，每个模块输出一行。检查**刻意仅做本地探测**（缓存 / 配置 DB 可打开、keyring 凭证存在）——绝不做网络调用，因此 `health` 快速且可离线使用。未覆写该检查的模块（search / auth / config）通过默认实现报告 `ok`。无论状态如何都渲染所有行；**退出码：全部健康为 0，任一模块 degraded 为 1**（便于脚本做门禁）。

| 命令 | 描述 | 用法 |
|------|------|------|
| `health` | 探测每个模块的本地健康状态 | `everyday health [--json]` |

**Text 输出**（每模块一行）：

```
$ everyday health
module    status  detail
------------------------
config    ok      ok
auth      ok      ok
...
```

**JSON 输出**（`--json`）：

```json
[{"detail":"ok","healthy":true,"module":"config"},{"detail":"ok","healthy":true,"module":"auth"},...]
```

实现见 [F012](adr/F012-architecture-deepening-phase.md) P3 生命周期钩子。

### sync — WebDAV 跨设备文件同步（v0.13.0 新增）

将真实用户数据做**文件级**双向同步到 WebDAV 目录（默认坚果云 `dav.jianguoyun.com`）：4 个用户 DB（`bookmark-<账户>.db` / `note-<账户>.db` / `todo-<账户>.db` / `memory.db`）+ `config.toml`。派生缓存（mail_cache / rss-items / timeline）永不参与同步。变更检测基于内容 hash（DB 先经 `VACUUM INTO` 生成一致快照，WAL 数据不会漏）；冲突用 **Last-Write-Wins + 双端冲突副本**——败方存为 `<名字>.conflict-<UTC 时间戳>.<扩展名>`，本地与远程各留一份，任何数据都不会丢。

| 命令 | 描述 | 用法 |
|------|------|------|
| `sync` | 双向同步（先拉后推）。首同步自动检测方向：远程目录空 → 全量推送；新设备（空配置模板）→ 全量拉取 | `everyday sync` |
| `--push-only` | 只上传本地变更 | `everyday sync --push-only` |
| `--pull-only` | 只下载远程变更 | `everyday sync --pull-only` |
| `--force` | 忽略 `sync-state.json`，全量重传本地 + 拉取远程独有文件 | `everyday sync --force` |

**配置**：先在 `config.toml` 配置账户（`[[webdav.accounts]]` — name / url / username），再把**应用密码**（非登录密码）存入系统钥匙串：

```
everyday auth login --module webdav --account personal
```

**自动同步（opt-in，默认关）**：账户 `auto_sync = true` 时，写命令（`bookmark add`、`note create`、`memory add` 等）成功后返回前会做一次 best-effort 变更文件推送。失败仅输出一行警告、绝不改变命令退出码；查询路径永不触发同步（[D003](adr/D003-auto-sync-cli-boundary.md)）。

同步状态存于 `sync-state.json`（与配置同目录，本身不参与同步）；删除它或传 `--force` 会从全量重传重建。设计：[D001](adr/D001-webdav-file-sync.md) / [D002](adr/D002-snapshot-hash-state.md) / [D003](adr/D003-auto-sync-cli-boundary.md)。

### mcp — 将 everyday 暴露为 MCP server（v0.15.0 新增）

把 `everyday` 变成 **Model Context Protocol（MCP）server**，走 stdio 传输。任何支持 MCP 的 agent（Claude Code / CodeBuddy / Cursor 等）用一行 `mcpServers` 配置即可连上，并把每个 `(module, action)` 作为名为 `<module>_<action>` 的 **tool** 调用——参数 schema 由 CLI 同源的 `module_arg_spec()` 生成，MCP 表面与 CLI 永不漂移。设计：[F014](adr/F014-mcp-module.md)，术语见 `../CONTEXT.md` §MCP。

| 命令 | 描述 | 用法 |
|------|------|------|
| `serve` | 运行 MCP stdio server（阻塞至 stdin 关闭后退出，退出码 0） | `everyday mcp serve` |
| `tools` | 打印投影出的 tool 清单 + JSON Schema（调试用） | `everyday mcp tools` |

**连接 MCP client**（例如 Claude Desktop 的 `claude_desktop_config.json`，或 Claude Code / CodeBuddy 对应的 `mcpServers` 块）：

```json
{
  "mcpServers": {
    "everyday": {
      "command": "everyday",
      "args": ["mcp", "serve"]
    }
  }
}
```

**注意**

- 每个 `(module, action)` 对应一个 tool：`mail_list`、`note_add`、`timeline_today` 等。`mcp` 模块自身不投影（不存在 `mcp_*` tools）。
- tool 返回 `--json` 渲染的输出；失败以 MCP `isError` 标记。可选 `account` 参数与 CLI `--account` 语义一致。
- server 是**长驻进程**：配置 / 数据库变更在**下次会话启动**时生效，会话中途不感知。
- stdout 专供 JSON-RPC；日志全部走 stderr。

### task — 命名命令执行与 cron 调度

执行 `[tasks.<name>]` 中配置的命令，不经过 shell；每次执行都记录到
`~/.config/everyday/task.db`。设计：[F017](adr/F017-task-module.md)。

| 命令 | 说明 | 用法 |
|------|------|------|
| `add` | 保留配置注释地新增任务；重名报错 | `everyday task add <name> --command <cmd> [--args <s>] [--allow-extra-args true\|false] [--timeout N] [--capture-output true\|false] [--schedule "cron"]` |
| `run` | 立即执行；仅 `allow_extra_args=true` 可追加 argv | `everyday task run <name> [-- extra...]` |
| `list` | 列出任务配置 | `everyday task list [--json]` |
| `remove` | 删除配置与调度状态，保留执行历史 | `everyday task remove <name>` |
| `history` | 查询最近执行记录 | `everyday task history <name> [--limit N] [--json]` |

**执行契约**

- `command` 直接派生；`args` 按空白拆分。无 shell、插值、管道或重定向；v1
  无法表达“单个参数自身包含空格”。
- 默认超时 60 秒，`timeout_secs = 0` 表示无限制。超时杀整棵进程树，记录
  `timeout`，everyday 退出 124。
- 手动文本模式实时透传 stdout/stderr 并镜像子进程退出码；`--json` 时子进程
  输出改走 stderr，stdout 仅一个 `{"_result": {...}}` 信封。Windows 控制台
  上透传经 OS 句柄写入，非 UTF-8 输出（如 GBK 的 `ipconfig`）原样显示不崩溃；
  落库记录按 UTF-8 有损转换。
- `capture_output=true` 分列保存 stdout/stderr，每流最多 64 KiB并带截断标记；
  定时执行无条件捕获。
- `schedule` 为本地时区标准 5 段 cron。daemon 用独立 30 秒循环检查，停机错过
  的窗口不补跑；`daemon run --once` 也执行一轮到期任务。

### daemon — 常驻自动同步（v0.17.0 新增）

后台常驻进程，按计划**拉取** mail / rss / timeline 事件，使 `timeline`、`search`、
`mail list` 查询无需显式 `--sync` 即可读到新鲜数据。daemon 是**唯一**允许周期性
拉取的角色——查询路径永不自动同步（L005），所以无论它是否运行，查询行为完全一致。
设计：[F016](adr/F016-daemon-sync-scheduler.md)；运维指南：
[daemon.md](daemon.md)。

| 命令 | 说明 | 用法 |
|------|------|------|
| `run` | 常驻同步循环：启动立即同步一次，之后每 `interval_seconds` 一个周期 | `everyday daemon run` |
| `run --once` | 只跑一个同步周期后退出（手动补拉 / 调试） | `everyday daemon run --once` |
| `run --sources mail,rss` | 覆盖 `[daemon].sources` 白名单 | `everyday daemon run --once --sources mail,rss` |
| `status` | 运行中 / 已停止（pid 存活校正）、上次周期、各源结果 | `everyday daemon status [--json]` |

**说明**

- 配置在 `config.toml` 的 `[daemon]` 节：`enabled`、`interval_seconds`（默认 900）、
  `sources`（空 = 全部）。`enabled = false` 时 `run` 拒绝启动（exit 1）。
- 每个周期（顺序执行，best-effort）：timeline 事件拉取 + mail 信封缓存同步
  （**服务器全部文件夹**）+ rss 缓存拉取。失败动作记录、不致命。
- 定时任务由独立 30 秒循环执行，不依赖同步间隔；输出捕获到 `task.db`。调度
  循环每轮重读 `config.toml`，`task add` / `task remove` 无需重启 daemon 即生效。
- `daemon run` 是**前台常驻进程**——用 OS 服务管理器安装（`nssm` / `launchd` /
  `systemd`），见 [daemon.md](daemon.md)。状态与日志位于
  `~/.config/everyday/daemon-state.json` 与 `daemon.log`。

## 输出模式

### Text 模式（默认）

适合终端直接查看，表格自动对齐：

```
$ everyday mail list --unread --limit 3
uid    unread  folder  date                          from              subject
-----------------------------------------------------------------------------
12345  true    INBOX   Wed, 8 Jul 2026 08:29 +0000  sender@x.com      Hello
12344  true    INBOX   Wed, 8 Jul 2026 07:15 +0000  boss@x.com        Weekly Report
12343  false   Drafts  Wed, 8 Jul 2026 06:00 +0000  me@x.com          Draft
```

### JSON 模式（`--json`）

输出纯净 JSON，无多余空白，适合程序化解析：

```bash
$ everyday mail list --unread --limit 2 --json
[{"uid":12345,"unread":true,"folder":"INBOX","date":"Wed, 8 Jul 2026 08:29:31 +0000","from":"sender@x.com","subject":"Hello"},{"uid":12344,"unread":true,"folder":"INBOX","date":"Wed, 8 Jul 2026 07:15:00 +0000","from":"boss@x.com","subject":"Weekly Report"}]
```

> `mail list` 输出为**保类型记录**：`uid` 是 JSON 数字、`unread` 是 JSON 布尔
> （此前一律字符串化）。见
> [F012](adr/F012-architecture-deepening-phase.md) P6。

### 错误输出

JSON 模式下错误格式：

```json
{"error": "AccountNotFound", "message": "mail account 'work'"}
```

退出码：成功 `0`，失败 `1`。

### 日志与级别

stderr 承载全部日志；stdout 只输出命令结果（[R001](adr/R001-thread-local-json-mode.md) 契约）。默认**安静**（WARN 级）：warning / error 可见，每次命令的进度日志静默。

| 参数 | 级别 | 效果 |
| --- | --- | --- |
| （无） | WARN | 默认：warning / error 可见，进度日志静默 |
| `-v` | INFO | 恢复中间件进度日志：text 模式 `[req] module action ok in 12ms`，JSON 模式 `{"_log": ...}` 行 |
| `-vv` | DEBUG | 预留（当前无 debug 输出） |

- auto_sync 成功通知（`warning: auto_sync_pushed: N file(s) pushed`）是 info 级，随 `-v` 出现；失败通知（`auto_sync_failed`）默认可见。
- JSON 模式下 stderr 为结构化行：`{"_log": ...}`（中间件进度）、`{"_warning": ...}`（部分失败：init 失败 / auto_sync / search provider / timeline sync）、`{"_error": ...}`（致命错误）。
- stdout 契约不变：命令结果（含 `--json`）是 stdout 的唯一内容。
