# Progress Log — Everyday

> 本文件仅保留**当前状态 + 核心决策（ADR）**。逐次会话的「已完成 / 测试结果 / 下一步」流水账已压缩；技术踩坑与 API 细节见 `findings.md`。

## 当前状态（2026-07-10）

- **v0.2.0 已发布**：tag `v0.2.0`，GitHub Release 附三平台（ubuntu/macos/windows）+ aarch64 macOS 预编译二进制。
- **模块**：5 个外部集成模块 **mail / cal / rss / note / todo** + `config` 均可用；初版 `fs` / `net` / `sys` 已整体移除。
- **质量门禁**：`cargo build` ✅、`cargo clippy --all-targets -- -D warnings` ✅ 零警告、`cargo test` ✅ 113 passed；CI（ubuntu/macos/windows + aarch64 mac）全绿。
- **文档**：README + `skills/everyday-cli/*` 与代码一致；范围与定位以 `agents.md`「范围与定位」为权威说明（原 PRD.md 已移除）。

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
- 与官方设计的有意偏差（核心 ADR）：**不新增 `AgentError` 变体**（复用 Auth/Network/Config，避免分裂错误分类）、**禁 `unwrap()`**（`NotionClient::new` 返回 `Result`）、**不引入 `toml_edit`**（用 `toml::Value` 局部编辑）、**note 暂不复用 `notion_client`**（避免回归，择机去重）。详见 `findings.md`「待办(todo)模块实现」。

### 2026-07-10 — CI / Release / v0.1.0
- `.github/workflows/ci.yml`（三平台 + aarch64 macOS，clippy `-D warnings` + `cargo fmt --check`）、`release.yml`（tag `v*` 触发，三平台 + aarch64 二进制）；发布 v0.1.0。

### 2026-07-10 — 移除过时的 PRD.md
- PRD 仍描述已删除的 fs/net/sys/剪贴板，与现实脱节；`git rm` 并清理全仓引用，范围改以 `agents.md` 为准（commit `fc14584`）。

### 2026-07-10 — 发布 v0.2.0
- 自 v0.1.0 以来的增量：`feat(todo)` Notion 待办模块 + 共享 notion-client SDK（commit `a721f5c`）、`fix(todo)` Status 改为 select 修复 Notion 过滤、`ci` 增加 aarch64-apple-darwin 到 CI + release 矩阵、移除过时 PRD.md、精简 progress/findings/task_plan 历史。
- 版本号 `0.1.0 → 0.2.0`（Cargo.toml + Cargo.lock 由 cargo 自动更新）；tag `v0.2.0` 触发 release workflow 构建三平台 + aarch64 macOS 预编译二进制。
