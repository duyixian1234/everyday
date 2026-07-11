# ADR 0011: Mail cache as envelope rows + UID watermark per folder

**Status:** Accepted
**Date:** 2026-07-11

## Context

`mail list` 优化需要本地存储以减少 IMAP 往返。备选形态有本质差异：

- **水位缓存**：仅存每文件夹的 `max_uid`，envelope 仍每次从 server 拉。库极小但 list 仍走网络。
- **完整 envelope 缓存**：每封邮件存一行 envelope 字段。`mail list` 默认查本地，零网络；增量 sync 拉新邮件写入本地。库可能 10-100MB。

用户明确选择后者："通过读取本地 sqlite 中的旧邮件时间戳，只获取更新的邮件" — 即"envelope 缓存 + 增量 sync"，日常 list 零网络。

新库 `~/.config/everyday/mail_cache.db` 与 `timeline.db` 解耦，独立存储。

## Decision

**mail_cache.db 双表设计：**

- `envelopes` 表：每封邮件一行，存储 envelope 完整字段。**主键 `(account, folder, uid)`**——IMAP UID 是 folder-scoped（同一封邮件在不同文件夹有不同 UID），复合主键准确表达这一约束。
  - 字段：`uid, folder, account, date (RFC3339), from, subject, flags (string), message_id, size, to, fetched_at (RFC3339 UTC)`
  - 索引：`(account, date DESC)` 用于 `mail list` 默认按日期降序查询
- `folder_state` 表：每文件夹一行水位元数据。**主键 `(account, folder)`**。
  - 字段：`uid_validity, max_uid, last_sync_at`
  - 用于增量 sync 的水位基线 + UIDVALIDITY 失效检测

**清理策略：K1，只追加不清理。**

- 邮件在 server 端被删除 / 跨文件夹移动 → 本地 envelope 仍留着（"幽灵邮件"）。
- 默认 `mail list --limit 20` 按日期排序，幽灵邮件通常已被新邮件挤出 limit，不影响日常体感。
- 不做 reconcile、不做 TTL、不做物理 DELETE。简单优先。
- 代价：数据库随时间增长，10 万封邮件约 30MB（每行 ~300B），可接受。

## Alternatives considered

### 水位缓存（仅 folder_state）

- `mail list` 仍走 server，每次拉 envelope。
- 用户明确反对——"只获取更新的邮件"意味着完整 envelope 缓存。

### 主键 `(account, message_id)`

- 全局唯一，同一封邮件跨文件夹只一行。
- 代价：sync 时无法用 UID 增量定位新邮件（必须先 fetch envelope 才能拿到 message_id）。
- 跨域复杂度高于收益。

### 软删除（active 标志 + 每次 sync 末尾 reconcile）

- 每次 sync 末尾 `UIDSEARCH UID 1:*` 拿全量 UID 列表比对，标 `active=0`。
- 正确性更好，但破坏"水位之上只查新邮件"的优化（每次 sync 多一次全量 SEARCH）。
- K1 + 默认 limit 20 已基本规避幽灵邮件问题，复杂度不必要。

### TTL 清理（fetched_at > 365 天物理删除）

- 粗暴，可能误删用户希望保留的旧邮件。
- 不符合 K1 简单优先。

## Consequences

- 数据库增长无界。10 万封 envelope 约 30MB；100 万封约 300MB。
  - 应对：未来若膨胀成为问题，加 `mail cache gc` 子命令 + 自动 TTL 配置项。
- `mail list` 默认 SQL：`SELECT uid, folder, date, from, subject FROM envelopes WHERE account = ? ORDER BY date DESC LIMIT 20`。毫秒级。
- 跨文件夹 `mail list` 时同一 `message_id` 可能多行（不同 folder）。可后续加 SQL `GROUP BY message_id`，当前不做。
- `folder_state` 必须与 `envelopes` 在同一事务中更新水位（确保崩溃后水位不超前于实际数据）。实施细节。
- 删除账号时需要级联清理两表（`DELETE FROM envelopes WHERE account = ?` + `DELETE FROM folder_state WHERE account = ?`）。