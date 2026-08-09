# ADR D002: Consistent snapshots + hash-driven state

**Status:** Accepted
**Date:** 2026-08-09

## Context

两个技术事实决定同步不能"直接 COPY 文件"或"用 mtime 判断变更"：

1. **sqlx 0.8 默认 WAL journal mode**——直接 COPY `.db` 会漏掉 WAL 中未 checkpoint 的数据（静默丢数据）。
2. **双设备时钟偏差**——mtime 无法权威判定"内容是否一致"；且 mtime 不同但内容相同会造成无效传输。

## Decision

- **推送**：每个 SQLite 文件先 `VACUUM INTO <tmp>` 生成一致快照（SQLite 原生、自动合并 WAL、对使用中的库安全），再 PUT 上传；`config.toml` 是纯文本，直接读取。
- **拉取**：GET 下载到 `<file>.tmp.<rand>` → SHA-256 校验 → `std::fs::rename` 原子替换（Windows 上为 MoveFileExW + MOVEFILE_REPLACE_EXISTING，可覆盖目标）。
- **状态**：本地 `sync-state.json` 记录每文件的 {本地 hash, 服务器 hash, 服务器 Last-Modified}。**不参与同步**；损坏用 `--force` 全量重传重建。
- **变更检测 = hash 权威**：本地 hash == 服务器 hash → skip（内容一致，无论 mtime 如何）。
- **冲突仲裁 = mtime / Last-Modified**：内容不一致时 LWW 需要时间戳；即便时钟偏差导致误判，被覆盖方进入冲突副本，不丢数据。
- **首同步检测**：远程目录空 → 推本地；本地文件全为默认模板（config 未配置 webdav、DB 未创建）→ 拉远程；否则正常双向。**关键陷阱**：新设备首次 pull 时本地 config.toml 是刚生成的空模板，mtime 比远程新，纯 LWW 会误判"本地赢"、用空配置覆盖远程完整配置——必须识别默认模板并特判为拉取。

## Alternatives considered

### 直接 COPY `.db`
拒绝：WAL 丢数据（见 Context）。CLI 进程退出后锁虽释放，但 WAL 内容风险仍在。

### mtime-only 变更检测
拒绝：时钟偏差导致内容漏判 / 误判；hash 计算对 KB-MB 级文件成本可忽略。

### sqlite backup API vs VACUUM INTO
sqlx 0.8 无 backup API；`VACUUM INTO` 是 SQL 语句，可经 sqlx execute 直发，零新依赖。代价是全量重写库——文件小（KB-MB 级），可接受。

## Consequences

- 每次推送全量重写快照（无增量传输优化）；文件小故成本可忽略。
- sync-state.json 是本地唯一状态源；删除它 = 下次 sync 全量重传（与 `--force` 同义），不破坏正确性，只增加传输量。
