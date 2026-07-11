# ADR 0012: Mail incremental sync via UID watermark + UIDVALIDITY detection

**Status:** Accepted
**Date:** 2026-07-11

## Context

mail_cache.db 的 envelope 缓存需要 sync 流程把 server 新邮件写入本地。备选同步策略：

- **日期窗口**：`SINCE (now - 30天)`。边界精度差（IMAP SINCE 只支持日期无时间），新邮件若早于 30 天前到达会漏掉。
- **UID 范围**：`UIDSEARCH UID <max_uid+1>:*`。UID 在 folder 内单调递增（除非 UIDVALIDITY 变更），是 RFC 3501 推荐的增量方式。
- **全量 SEARCH**：每次都 `UIDSEARCH ALL`。失去优化意义。

IMAP 协议规定 UIDVALIDITY 字段——文件夹重建时 UIDVALIDITY 改变，所有旧 UID 失效。RFC 3501 §2.3.1.1 要求实现必须能检测 UIDVALIDITY 变化。

首次使用（无水位）时 `UID max_uid+1:*` 等价于 ALL（max_uid=0 时 `UID 1:*` 是所有邮件）——全量慢，但只一次。

## Decision

**单文件夹 sync 流程：**

1. `SELECT folder` 拿 `UIDVALIDITY`（async-imap `Session::select` 返回 `Mailbox` 结构含 `uid_validity`）。
2. 读本地 `folder_state`：得到 `(uid_validity, max_uid, last_sync_at)`。
3. **UIDVALIDITY 不一致**：
   - `DELETE FROM envelopes WHERE account = ? AND folder = ?`
   - `UPDATE folder_state SET uid_validity = ?, max_uid = 0 WHERE ...`
   - 回退到全量（`UIDSEARCH UID 1:*`）
4. **水位为 0（首次）**：`UIDSEARCH UID 1:*` 全量拉所有 UID。
5. **正常增量**：`UIDSEARCH UID <max_uid+1>:*` 拿新邮件 UID 列表。
6. `UID FETCH <uid1>,<uid2>,... (UID ENVELOPE FLAGS RFC822.SIZE)` 批量拉 envelope。
7. 写入 envelopes 表。
8. 更新 `folder_state.max_uid = max(new_uids)`、`last_sync_at = now()`。**水位更新与 envelope 写入同一事务**——崩溃后水位不超前于实际数据。

**首次接受慢**：水位为 0 时等价全量。10k 邮件 `UIDSEARCH UID 1:*` 在 Gmail 约 1-3 秒。一次性成本。

## Alternatives considered

### 日期窗口 SINCE <date>

- IMAP SINCE 只支持日期，精度差。
- 新邮件若早于查询日期（如跨时区邮件晚到），会漏掉。
- 不如 UID 精确。

### 不检测 UIDVALIDITY

- 服务器重建文件夹后，旧 UID 被复用为不同邮件。本地 envelope 错乱。
- 必须检测。RFC 3501 强制。

### 写入与水位分离（无事务）

- envelope 先写，水位最后写。崩溃可能水位不更新（下次重拉相同邮件，幂等，无害）。
- envelope 写入后水位已写但 envelope 部分缺失——可能水位超前，下次漏邮件。
- 同一事务解决。

### 用 SQLite WAL + 异步水位更新

- 性能更好但增加复杂度。
- envelope 写入频率低（每个 folder sync 一次），事务开销可忽略。

## Consequences

- `folder_state.last_sync_at` 是 staleness 检查的依据（见 ADR 0013）。
- UIDVALIDITY 变更检测需要在每个文件夹 sync 开头 SELECT 一次——多一次 IMAP 往返。SELECT 是最便宜的 IMAP 命令，可忽略。
- max_uid 永远递增；server 端 expunge 不影响水位（UIDSEARCH `max_uid+1:*` 仍正确返回新邮件）。
- 跨文件夹移动的语义：原 folder UID 仍存在（IMAP 移动是 copy + delete，新 folder 分配新 UID）；sync 时原 folder max_uid 不变，新 folder 出现新 UID（被 max_uid+1:* 捕获）。原 folder 缓存里该 UID 永远留着（K1 简单策略），不影响日常 list。
- 失败语义：单文件夹 sync 失败（SELECT / SEARCH / FETCH 任意一步错）→ 跳过该文件夹，**不更新水位**，下次 sync 重试（best-effort，与 Timeline ADR 0009 一致）。