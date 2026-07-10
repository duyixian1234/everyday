# everyday — Agent 用户入门

`everyday` 是一个 Rust 编写的本地 CLI 工具集，作为 AI Agent 的"数字双手"，统一命令结构，覆盖邮件（IMAP/SMTP）、日历（CalDAV）、RSS 订阅、笔记（Notion）、待办（Notion）、配置等外部集成场景。

```
everyday <module> <action> [options] [--json] [--account NAME]
```

## 给 Agent 的指引

- **要操作 everyday 命令**，加载 **`everyday-cli`** skill（`everyday-cli/SKILL.md`）。它包含触发场景、必守规则与常见任务示例。
- **完整命令表、选项与输出 schema** 在 `everyday-cli/references/COMMANDS.md`，按需读取。
- **交互一律加 `--json`**，拿到结构化数据后再处理；AI 不应解析人类表格。
- **凭证走系统密钥环**（`everyday/<module>/<account>`），密码既不存配置文件，也不作为命令行参数传入。

## 模块实现状态

| 模块 | 状态 |
|------|------|
| `config` · `mail` · `cal` · `rss` · `note` · `todo` | ✅ 可用 |

> 本文件面向 Agent 用户，精简介绍；人类可读的完整文档见仓库根 `README.md`，协作规范见 `agents.md`。

## 安装 everyday

- **预编译二进制**（Linux / macOS / Windows x86_64）：[GitHub Releases](https://github.com/duyixian1234/everyday/releases)，每个 `v*` tag 自动发布，下载解压后把 `everyday` 加入 `PATH`。
- **从源码**：`cargo install --git https://github.com/duyixian1234/everyday.git`，或 `git clone` 后 `cargo build --release`。
- 验证：`everyday --version`。完整安装步骤见仓库根 `README.md`。
