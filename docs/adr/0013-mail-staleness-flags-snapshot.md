# ADR 0013: Mail list staleness-based auto-sync + flags snapshot + search bypass

**Status:** Accepted
**Date:** 2026-07-11

## Context

mail_cache.db 缓存 envelope 后，`mail list` 何时触发 sync？三个候选：

- **每次自动 sync**：默认 `mail list` 触发一次 sync 再查本地。破坏 Timeline ADR 0005（no-auto-sync）原则，且每次 list 都有 IMAP 往返。
- **永远不自动 sync**：用户必须记得 `mail sync` / `--sync`。agent 友好度差。
- **Staleness 自动 sync**：默认查本地；若 `folder_state.last_sync_at` 距今 > 阈值，自动 sync 一次。混合方案。

flags（已读/未读）是 envelope 缓存的字段之一。用户在 web 端 / 其他客户端改 flags 后，本地缓存多快跟上？

search（关键词搜索）与 list 行为不同：search 不知道邮件 UID，无法用水位增量。候选：

- **search 走本地 envelope LIKE**：速度快但只能搜 subject/from，搜不到 body。
- **search 走 IMAP 直连**：保留完整 IMAP SEARCH 语义（含 BODY/TEXT），慢但准确。
- **混合**：本地搜不到再回退 IMAP。复杂。

## Decision

**staleness 自动 sync：**

- 默认 `mail list`：检查所有目标 folder 的 `folder_state.last_sync_at`，若任一 folder 距今 > **15 分钟**，触发一次 sync（增量 UIDSEARCH）。否则纯本地查询。
- `--sync` flag：强制忽略 staleness，立即触发 sync。
- 阈值 15 分钟**写死**，不通过 flag 或 config 暴露。
- 触发范围：仅 `mail list`。`mail read`、`mail send` 不触发 sync。

**flags 缓存即真相（F1）：**

- `envelopes.flags` 是 sync 时刻的快照。
- `mail list --unread` 反映 sync 时的已读状态，最坏 15 分钟滞后。
- 用户在 web 端标记已读后 15 分钟内 `--unread` 可能仍显示未读——明确边界，可接受。

**search 走 IMAP 直连，不读缓存：**

- `mail search --query Q` 完全保持当前实现（`collect_across_folders` 直连 IMAP `SEARCH TEXT "Q"`）。
- 与 list 行为不对称：list 走本地缓存（瞬间），search 走 server（慢）。
- 未来可加本地 LIKE 搜索作为 `mail search --cached` flag（不在本 ADR 范围）。

## Alternatives considered

### 每次 list 自动 sync

- 破坏 Timeline `no-auto-sync` 原则。
- 频繁 list 触发重复 sync，浪费 IMAP 往返。

### 仅 `--sync` flag（无自动）

- agent 必须主动管理 sync 状态，不友好。

### staleness 阈值可配置

- 增加配置面（`[mail] staleness_minutes`）。
- 15 分钟是经验合理值，多数 agent 调用频率匹配；微调收益低。
- 未来若用户反馈频繁 sync 浪费或 lag，再暴露。

### flags 每次 list 前重拉

- `UID FETCH uid FLAGS` 单独轻量调用。
- 破坏"零网络 list"目标。
- 15 分钟滞后可接受，不值得增加复杂度。

### flags sync 时立即单独 fetch

- 当前实现就是 sync 时拉 ENVELOPE FLAGS，已经是最新。
- 增量同步阶段无优化空间。

### search 走本地 LIKE

- 仅支持 subject/from 字段。
- IMAP `SEARCH TEXT "Q"` 可搜 subject + body + header 等多字段。
- 用户用 `--query` 期望完整搜索能力。

### search 混合（本地优先 + IMAP 回退）

- 复杂，本地 LIKE + IMAP SEARCH 语义不一致。
- 用户预期 search 是"完整搜索"，降级语义模糊。

## Consequences

- `mail list` 行为对 agent 是可预测的：本地查 < 100ms（首次可能自动 sync 一次，1-3 秒）。
- 文本输出需说明本次是否触发 sync（如 `synced N folders (M new envelopes), listed K from cache`）——非 JSON 模式可读性。
- `--unread` 15 分钟滞后需在文档中明确说明。
- `mail search` 与 `mail list` 的不对称需在 README / help 中说明。
- 未来扩展路径：
  - `mail list --full` 跳过缓存，全量 IMAP 拉（兜底）
  - `mail search --cached` 本地 LIKE 搜索
  - `mail cache gc` 物理清理过期 envelope（应对未来膨胀）
- 时间戳统一 UTC 存储（与 Timeline ADR 0006 一致），query 时按本地时区转换。