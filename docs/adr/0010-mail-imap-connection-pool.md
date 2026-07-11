# ADR 0010: Mail list IMAP connection pool with semaphore

**Status:** Accepted
**Date:** 2026-07-11

## Context

`everyday mail list` 当前的实现 `collect_across_folders` 在单个 IMAP session 中串行遍历文件夹（`SELECT folder → UID SEARCH → UID FETCH → SELECT next folder → ...`）。对有 N 个文件夹的邮箱，单次 list 至少触发 3N 次 IMAP round-trip。当 N = 10-30（如 Gmail / 网易邮箱带大量自定义文件夹）时，list 在普通网络下耗时数秒，agent 调用体感明显。

用户场景：
- AI Agent 每隔几分钟调用 `mail list --json` 探测新邮件
- 邮箱有 10+ 文件夹（INBOX + 已发送 + 草稿 + 多个标签 / 自定义目录）
- 单次 list 必须快（理想 < 500ms 增量，< 3s 首次）

IMAP 协议本身是状态机——一个 session 同一时刻只能 `SELECT` 一个文件夹，无法在同 session 内"并行 SELECT A 与 SELECT B"。

## Decision

**采用固定大小连接池 + 信号量分发模型：M = 4 个并发的 IMAP session，N 个文件夹通过 tokio semaphore 抢占。**

- 启动时建立 4 个 IMAP session（共享同一份 keyring 密码）。
- 每次 `mail list` 触发 sync 时，对每个文件夹：acquire 信号量 → 在池中任一空闲 session 上 `SELECT folder → UID SEARCH → UID FETCH` → release。
- 4 是固定值，不通过 flag 或 config 暴露。
- 单 session 复用减少 TLS 握手 + IMAP LOGIN 开销；并发上限避免触发服务器 ban。

## Alternatives considered

### A. 单 session 异步流水

- 同 session 内 `SELECT A → SEARCH A → SELECT B → SEARCH B → ...` 串行执行。
- 仅省去多连接 TLS 开销，不省 IMAP 往返——N 文件夹仍要 3N 次往返，与优化目标冲突。

### B. 单 session 多路复用（IMAP 命令流水线）

- IMAP 协议本身不支持命令流水线（同步请求-响应）。RFC 3501 明确要求每个 tag 命令等 tagged response 才能发下一条。
- 不存在此方案。

### C. 无限并发连接

- 每个文件夹独立 session。
- 服务器典型上限 5-15 个并发连接；过多触发连接拒绝 / IP ban。
- 缺少信号量控制无 backpressure。

### D. 复用 Timeline `sync_state` 水位

- 用 timeline.db 的水位判断哪些文件夹"已知"，跳过无变化的文件夹。
- 跨域耦合：mail list 行为依赖 Timeline 已同步过的状态——`mail list` 应独立。
- 已在前置讨论中排除。

## Consequences

- 实现上需要新增 `src/modules/email_pool.rs`：管理 4 个 session 的 `Vec<Mutex<ImapSession>>` + `tokio::Semaphore::new(4)`。
- session 失败需要重连：当前 session 任意命令失败 → 重建该 session + 重试一次。整体 best-effort：一个文件夹失败不阻塞其他。
- 4 个 session 同时 TLS 握手在首次 list 时增加 4 × TLS 握手时间（~1-2s）。后续增量 list 复用已建立的连接，仅 LIST（拿文件夹列表）+ 各文件夹 SELECT 有开销。
- M = 4 是经验值（Gmail / Outlook / 网易邮箱 / QQ 邮箱实测均不触发限流）。后续若发现真实限流，再考虑暴露 `mail.imap_pool_size` 配置项。
- 内存开销：4 个 session + 各自 TCP/TLS 缓冲区，约数十 KB。