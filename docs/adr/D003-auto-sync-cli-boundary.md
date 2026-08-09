# ADR D003: auto_sync — the CLI process boundary

**Status:** Accepted
**Date:** 2026-08-09

## Context

用户希望可选"写后自动推送"。但 everyday 是**短生命周期 CLI 进程**：`tokio::spawn` 的后台任务在 main 返回时**不保证执行完成**——CLI 里不存在真正的 fire-and-forget。同时 [F009](F009-performance-budget.md) 冷启动 < 100ms 与 [L005](L005-no-auto-sync.md)"查询永不自动同步"是项目铁律。

## Decision

- **auto_sync 默认关闭**（`webdav.accounts[].auto_sync = true` 才开启，opt-in）。
- **开启时**：写操作命令（bookmark add / memory add / note create / todo add 等）执行完毕后、输出返回前，做一次 best-effort 推送：
  - 只推有变更的文件（hash 检测，见 [D002](D002-snapshot-hash-state.md)）；
  - **同步等待**推送完成（诚实语义：CLI 无真后台，不等完就返回 = 静默丢变更）；
  - 失败**不改变命令退出码**，仅记录 ops-log 并输出一行警告。
- **查询路径永不触发 sync**（拉取永远显式 `everyday sync`）——延续 L005，无例外。
- 推送网络超时设短上限（默认 10s）：离线时写命令至多 +10s 延迟且最终成功退出。

## Alternatives considered

### 真 fire-and-forget（tokio::spawn 后立即返回）
拒绝：CLI 进程退出即杀后台任务，推送未完成 = 静默丢变更。除非引入常驻 daemon 进程（复杂度不可接受）。

### 定时自动拉取 / 推送
拒绝：需 daemon 或 OS 定时器；与"查询永不自动同步"冲突。

### auto_sync 默认开启
拒绝：未配置 webdav 的用户不该有隐式网络行为；离线环境写命令被拖慢不可接受。

## Consequences

- 开启 auto_sync 后写命令末尾增加一次网络往返（离线时最多 +10s 超时）。
- 语义清晰：sync 始终"显式可控"，auto_sync 只是写命令的收尾钩子。
- 未来若引入常驻进程（daemon / 系统托盘），可升级为真后台推送，不破坏本 ADR 的显式语义。
