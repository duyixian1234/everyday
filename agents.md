# agents.md — Everyday AI Agent 协作规范

> 这是 AI Agent 与人类协作者的项目入口。先读这一段，再按需跳到 `.rules/`
> 看具体约定，到 [docs/adr/](./docs/adr/) 读决策。

## 项目概览

**Everyday** 是一个 Rust 编写的本地 CLI 工具集，作为 AI Agent 的"数字双手"。
统一命令结构 `everyday <module> <action> [options]`，支持 Text / JSON 双输出，
JSON 为 AI 交互主模式。

- **语言：** Rust (edition 2024)
- **二进制名：** `everyday`
- **目标平台：** Linux / macOS / Windows（含 Apple Silicon aarch64）
- **异步运行时：** `tokio`
- **规划文件：** `task_plan.md`（阶段 + Errors）/ `progress.md`（当前状态 + ADR
  时间序索引）/ `findings.md`（ADR 主题索引）。详见 [文档约定](#文档约定)。

## 何时使用本仓库

引用 [F003](./docs/adr/F003-module-scope-external-integration.md)：Everyday 只
封装**外部集成协议 / 状态 / 凭证**——IMAP、SMTP、CalDAV、RSS、Notion、bookmark
本地 DB、Timeline 事件层——不封装 `fs` / `net` / `sys` / 剪贴板等通用能力（代理
用 shell / `curl` / `fd` / `rg` 即可直接完成）。模块提案必须先回答"封装了什么
shell 做不到的事"。

当前模块：

| 模块 | 范围 | 决策入口 |
| --- | --- | --- |
| `mail` | IMAP 列表 / 读 / 搜索；SMTP 发送；envelope 缓存 | [M001](./docs/adr/M001-imap-stack.md) – [M005](./docs/adr/M005-staleness-auto-sync.md) |
| `cal` | CalDAV 日历：列出 / 创建 / 删除 | [C001](./docs/adr/C001-caldav-stack.md) – [C003](./docs/adr/C003-cal-provider-window-filter.md) |
| `rss` | RSS / Atom 订阅聚合 | [F008](./docs/adr/F008-rss-module.md) |
| `note` | Notion 笔记；本地 SQLite provider 默认 | [N001](./docs/adr/N001-notion-note-module.md)，[F005](./docs/adr/F005-default-provider-local.md) |
| `todo` | Notion 待办（add / start / complete / delete） | [T001](./docs/adr/T001-notion-todo-module.md), [T002](./docs/adr/T002-todo-delete-action.md) |
| `bookmark` | 书签：本地 SQLite 默认，Notion 备选 | [B001](./docs/adr/B001-bookmark-dual-provider.md) |
| `timeline` | 跨模块统一事件层（append-only log） | [L001](./docs/adr/L001-append-only-event-log.md) – [L013](./docs/adr/L013-from-explicit-error.md) |
| `config` | 配置查看 / 修改；走 Executor trait | [R012](./docs/adr/R012-config-executor-trait.md) |

## 文档约定

Agents 与协作者在改代码前，按以下顺序读：

1. 本文件（入口）
2. [`task_plan.md`](./task_plan.md) 当前阶段
3. 相关 [`.rules/*.md`](./.rules/RULES.md) 主题规则
4. 相关 ADR（设计决策）

每个文档的"产权"边界：

| 文档 | 内容 |
| --- | --- |
| `agents.md` | 项目入口、技术栈、模块清单、文档约定 |
| [`.rules/`](./.rules/RULES.md) | 非决策类约定（workflow / style / testing / security / commit / justfile / crate 踩坑 / **注释策略**） |
| [`docs/adr/`](./docs/adr/README.md) | 每个架构决策的"上下文 / 决策 / 备选 / 影响"（F/M/C/N/T/B/L/R 系列） |
| [`CONTEXT.md`](./CONTEXT.md) | 领域术语表（仅定义，不涉及实现） |
| [`task_plan.md`](./task_plan.md) | 阶段 + 错误表 + 设计决策摘要 |
| [`progress.md`](./progress.md) | 当前状态 + ADR 时间序索引 + 发版流水 |
| [`findings.md`](./findings.md) | ADR 主题索引（纯索引，无叙述） |
| `README.md` / `README_ZH.md` / `skills/` | 终端用户与 Agent 用户文档 |

跨文档引用都用相对路径。每次提交后跑 `just check-links` 验证引用未腐烂（见
[`.rules/06-justfile.md`](./.rules/06-justfile.md)）。

## 完成一个任务后

按 [`.rules/01-workflow.md`](./.rules/01-workflow.md) §"Finishing a task" 执行
五步：

1. 质量门禁 `just ci`（format / clippy / test / build 全绿）
2. `just check-links`（跨文档引用完整性）
3. **ADR 抽取**——把决策性内容从 `progress.md` / `findings.md` 推到 ADR，
   `findings.md` 与 `progress.md` 仅剩索引
4. `git commit` 按 [`.rules/05-commit.md`](./.rules/05-commit.md)
5. 更新 `progress.md` 的 ADR 时间序索引

发版流程见
[`.rules/01-workflow.md`](./.rules/01-workflow.md) §"Release (runbook summary)"。
