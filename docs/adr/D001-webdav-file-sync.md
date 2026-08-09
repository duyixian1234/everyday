# ADR D001: WebDAV file-level sync — scope, semantics, conflict

**Status:** Accepted
**Date:** 2026-08-09

## Context

everyday 的真实用户数据（bookmark / memory / note / todo 四个 SQLite 库 + config.toml）只存在本机：无备份、无跨设备通道。用户有多设备（家中 / 公司）日常使用的需求。现有同步只覆盖 Timeline 的事件拉取（L-series），是"源 → 本地"单向增量，不解决"本地文件本身跨设备复制"。

同时项目有硬约束：冷启动 < 100ms（[F009](F009-performance-budget.md)）、查询路径永不自动同步（[L005](L005-no-auto-sync.md)）。

## Decision

**新增 `sync` 模块，对 5 个数据文件做双向文件级同步到 WebDAV（RFC 4918，默认坚果云 `dav.jianguoyun.com`）。**

- **范围**：`bookmark-<account>.db` / `memory.db` / `note-<account>.db` / `todo-<account>.db` / `config.toml`。派生缓存（mail_cache / rss-items / timeline / ops-log）不同步——可从源重建，同步只会制造冲突。
- **方向**：双向，先拉后推（pull-then-push）收敛；`--push-only` / `--pull-only` 显式覆盖方向；`--force` 忽略 sync-state 全量重传。
- **冲突**：文件级 Last-Write-Wins。败方存档为 `xxx.conflict-<UTC ts>.db`，保留本地**并上传远程**——被覆盖数据的唯一恢复来源。
- **认证**：走 auth 模块（[R013](R013-auth-module-consolidation.md) / [R015](R015-auth-credential-io.md) 惯例）：`everyday auth login --module webdav --account <name>`，应用密码存 keyring（`everyday/webdav/<account>`），config 只存 url/username。凭据永不进入 WebDAV 同步内容。
- **加密**：V1 明文上传（数据敏感度评估为中低；明文保证"服务器即真相"的最简恢复路径；加密 deferred，见 Alternatives）。
- **查询永不自动同步**（延续 L005）：`auto_sync`（默认关）只在写命令末尾 best-effort 推送，见 [D003](D003-auto-sync-cli-boundary.md)。

## Alternatives considered

### 行级合并（row-level merge）
按主键对 bookmark / memory / note / todo 做 union，双端各自新增自动合并。拒绝（V1）：每个库需专属 merge 逻辑且要求 schema 稳定主键；合并结果幂等需要同步标记字段，工程量 ×3-4。文件级 LWW + 冲突副本零数据丢失；双端并发新增的代价是手动处理冲突副本——频率低、可接受。保留为 V2 候选。

### 加密上传（AES-256-GCM）
拒绝（V1）：把恢复路径绑定到本机 keyring 密钥，重装 / 换机多一个密钥迁移环节；而明文下 WebDAV 文件即真相，任何工具可取回。`encrypt = true` 留作未来配置项。

### 专用 webdav crate（webdav-client）
拒绝：只需要 MKCOL / PUT / GET / PROPFIND 四个方法，reqwest 0.12 已具备；外部 crate 的维护状态与依赖面不可控，手写薄封装约 200 行。

## Consequences

- 双端各自新增条目会触发文件级冲突，败方落入冲突副本，需人工处理（副本在任一端可见）。
- 明文存放于第三方服务器——用户自决的风险（国内云盘明文放书签 / 笔记）。
- `sync` 为新模块：clap 子命令树（[F007](F007-clap-subcommand-tree.md)）、ModuleRegistry 注册、ops-log 记录均需接线。
