# Everyday CLI

- [English](README.md) · [中文](README_ZH.md)

---


> The Rust-powered hands for your AI Agent.

**语言 / Language：** [English](README.md) · **简体中文**

`everyday` 是一款高性能、内存安全的本地 CLI 工具集，用 Rust 编写。它作为 AI Agent 的"数字双手"，统一命令结构，覆盖邮件、日历、RSS 订阅、笔记（本地 SQLite）、待办（本地 SQLite）、书签（本地 SQLite）、以及 Agent 自身的结构化记忆笔记本等场景，支持 Text / JSON 双输出模式。

## 特性

- **统一命令结构**：`everyday <module> <action> [options]`，学习成本低
- **双输出模式**：默认 Text（人类可读表格），`--json` 切换为纯净 JSON（AI 交互主模式）
- **多账户支持**：每个模块支持多个命名账户，`--account` 灵活切换
- **凭证安全**：密码走系统密钥环（macOS Keychain / Windows Credential Manager / Linux Secret Service），绝不落盘
- **跨平台**：Windows / macOS / Linux
- **高性能**：冷启动 < 100ms，异步运行时（tokio），内存安全


## 安装

可通过一键安装脚本（自动取 latest）、从 [GitHub Releases](https://github.com/duyixian1234/everyday/releases)
下载预编译二进制、从源码构建或通过 cargo 安装：

```bash
# macOS / Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/duyixian1234/everyday/releases/latest/download/everyday-installer.sh | sh
```

```powershell
# Windows（PowerShell）
powershell -ExecutionPolicy Bypass -c "irm https://github.com/duyixian1234/everyday/releases/latest/download/everyday-installer.ps1 | iex"
```

```bash
cargo install --git https://github.com/duyixian1234/everyday.git

everyday --version   # 验证安装
```

> 完整各平台步骤、资产表与校验和见
> [docs/installation_zh.md](docs/installation_zh.md)。

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
everyday auth login --module mail --account work
# 提示输入密码，存入系统密钥环（不落盘）
```

### 4. 开始使用

```bash
# 列出未读邮件
everyday mail list --unread

# JSON 模式（AI 友好）
everyday mail list --unread --limit 10 --json
```


## 命令速览

| 模块 | 用途 | 入口 |
|------|------|------|
| `config` | 配置管理 | `everyday config` |
| `mail` | 邮件管理 | `everyday mail` |
| `cal` | 日历管理（CalDAV） | `everyday cal` |
| `rss` | RSS/Atom 订阅 | `everyday rss` |
| `note` | 笔记与知识库 | `everyday note` |
| `todo` | 待办任务 | `everyday todo` |
| `bookmark` | 书签 | `everyday bookmark` |
| `auth` | 凭证生命周期 | `everyday auth` |
| `timeline` | 统一事件流 | `everyday timeline` |
| `search` | 跨模块统一搜索 | `everyday search` |
| `memory` | 结构化记忆笔记本 | `everyday memory` |
| `health` | 模块健康检查 | `everyday health` |
| `sync` | WebDAV 跨设备同步 | `everyday sync` |
| `mcp` | 将 everyday 暴露为 MCP server | `everyday mcp` |
| `daemon` | 常驻自动同步 | `everyday daemon` |
| `task` | 命名命令执行、历史与 cron 调度 | `everyday task` |

> 完整模块命令表、选项与输出模式见 [docs/commands_zh.md](docs/commands_zh.md)。

## 文档

- [安装](docs/installation_zh.md) — 安装脚本、预编译二进制、源码构建
- [命令参考](docs/commands_zh.md) — 各模块完整命令表与输出模式
- [配置](docs/configuration_zh.md) — config.toml、凭证安全、多账户
- [使用示例](docs/examples_zh.md) — 各模块可复制示例
- [开发](docs/development_zh.md) — 技术栈、构建、架构、实现状态
- [daemon 运维指南](docs/daemon.md) — 安装为系统服务（nssm / launchd / systemd）
- [设计决策](docs/adr/) — F/M/C/N/T/B/L/R 系列 ADR
- [协作指南](agents.md) — 贡献者工作流

## 许可证

MIT
