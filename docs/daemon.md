# everyday daemon — 常驻自动同步

> 设计决策见 [ADR F016](./adr/F016-daemon-sync-scheduler.md)。本文是部署运维指南：
> 安装为系统服务、配置、命令、日志与状态文件。

## 概述

`everyday daemon` 是唯一允许**周期性自动拉取**的角色。常驻期间，它按固定间隔
把 mail / rss / timeline 事件拉进本地缓存，使 `timeline` / `search` / `mail list`
查询随时拿到新鲜数据——AI 助手无需手动 `--sync`。

**查询语义不变**：查询路径永不触发同步（L005），daemon 运行与否都不改变查询行为。
daemon 只是"替你把显式 sync 按时跑了"。

## 安装为系统服务

三平台各给出一种推荐方案。原则：**前台常驻进程交给 OS 服务管理器**，`everyday`
本身不做 daemonize。

### Windows — nssm（推荐）

```powershell
# 1. 安装 nssm（https://nssm.cc），然后：
nssm install everyday "C:\Path\to\everyday.exe" daemon run
# 若需自定义间隔/过滤：
nssm install everyday "C:\Path\to\everyday.exe" daemon run
nssm set everyday AppParameters daemon run
# 2. 查看/启动：
nssm status everyday
nssm start everyday
# 3. 卸载：
nssm remove everyday confirm
```

替代方案（任务计划程序）：创建计划任务，操作 = 启动 `everyday.exe`，
参数 = `daemon run`，触发器 = "系统启动时"，设置 = 允许在无人登录时运行。
注意任务计划程序会以独立会话启动进程，日志/状态文件路径不受影响（都在
`~/.config/everyday/` 下）。

### macOS — launchd（plist 模板）

`~/Library/LaunchAgents/com.duyixian.everyday.daemon.plist`：

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.duyixian.everyday.daemon</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/local/bin/everyday</string>
    <string>daemon</string>
    <string>run</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>/tmp/everyday-daemon.stdout.log</string>
  <key>StandardErrorPath</key>
  <string>/tmp/everyday-daemon.stderr.log</string>
</dict>
</plist>
```

```bash
launchctl load ~/Library/LaunchAgents/com.duyixian.everyday.daemon.plist
launchctl start com.duyixian.everyday.daemon
launchctl unload ~/Library/LaunchAgents/com.duyixian.everyday.daemon.plist  # 停止
```

`KeepAlive` 使进程崩溃后自动重启；`enabled = false`（config）可让崩溃重启循环
立即报错退出，避免空转。

### Linux — systemd（unit 模板）

`/etc/systemd/system/everyday-daemon.service`：

```ini
[Unit]
Description=everyday daemon auto-sync
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/everyday daemon run
Restart=on-failure
RestartSec=10
# 若 everyday 以普通用户运行，取消注释并改对路径：
# User=yixian
# Environment=HOME=/home/yixian

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now everyday-daemon
sudo systemctl status everyday-daemon
sudo systemctl restart everyday-daemon
```

## 配置 `[daemon]`

```toml
[daemon]
enabled = true          # false 时 `daemon run` 报错退出（exit 1）
interval_seconds = 900  # 一个周期完成后 sleep 的秒数（默认 15 分钟）
sources = []            # 空 = 全部；白名单 e.g. ["mail","rss"]
```

- 间隔是"周期完成后 sleep"语义：同步耗时不会叠加进间隔，也不会触发追赶。
- `sources` 白名单同时控制 timeline provider 与对应缓存动作（含 mail 全文件夹
  同步 / rss 拉取）。本地 provider（todo/note/bookmark）零成本，始终同步。

## 命令

| 命令 | 说明 |
|---|---|
| `everyday daemon run` | 前台常驻：启动立即同步一次，然后每 `interval_seconds` 一个周期 |
| `everyday daemon run --once` | 只跑一个周期后退出（同步汇总输出到 stdout；手动补拉/调试用） |
| `everyday daemon run --once --sources mail,rss` | 覆盖配置的 sources，只同步指定源 |
| `everyday daemon status` | 运行中 / 已启用 / 上次周期 / 各源结果 |
| `everyday daemon status --json` | 同上，JSON 输出（命令结果本体，非 `_log` 形状） |

退出码：0 = 正常退出（`--once` 完成 / 信号优雅退出）；1 = `enabled=false`、
已有实例运行、状态文件写入失败等。

## 日志与状态文件

| 文件 | 用途 | 清理 |
|---|---|---|
| `~/.config/everyday/daemon.log` | daemon 文件日志，固定 INFO 级别，追加写 | 手动删除即可（下一周期重建） |
| `~/.config/everyday/daemon-state.json` | 运行状态 + 各源最近结果 | 手动删除后 `status` 显示"未运行"；运行时会重建 |

- 常驻期间 stdout 完全静默；stderr 默认 WARN 静音，`-v`/`-vv` 分级（交互调试用）。
- `daemon.log` 是主日志（固定 INFO，不随 `-v`），每次写入即开即关（无需显式关闭）。
- 停止 daemon 后状态文件保留（`running=false` + `exit_at`/`exit_ok`），可回看
  "上次同步到几点"。

### daemon-state.json 结构（schema 与实现一致）

```json
{
  "pid": 12345,
  "running": true,
  "enabled": true,
  "interval_seconds": 900,
  "started_at": "RFC3339",
  "last_cycle_at": "RFC3339",
  "cycles": 1,
  "last_cycle_ok": true,
  "exit_at": null,
  "exit_ok": null,
  "sources": {
    "timeline": { "ok": true, "events": 12, "error": null },
    "mail":     { "ok": true, "folders": 8, "envelopes": 34, "error": null },
    "rss":      { "ok": true, "items": 5, "error": null }
  }
}
```

- 写盘时机：启动（pid / running=true / started_at）/ 每周期结束（last_cycle_at /
  cycles / last_cycle_ok / sources）/ 退出（running=false + exit_at / exit_ok，
  sources 保留最近结果）。原子写（临时文件 + rename）。
- `status` 的 `running` 判定 = **pid 存活探测**（Linux `/proc`、macOS `kill -0`、
  Windows `tasklist /FI` CSV 引号 PID 匹配）：文件写着 `running=true` 但 pid 已死
  → 报 `stopped`（stale 状态）。
- 防重入：`daemon run` 启动时若状态文件含存活 pid → 报错 + exit 1。

### 优雅退出

`--once` 完成 / SIGINT（Ctrl+C）/ SIGTERM（Unix 服务管理器）汇合到
`graceful_shutdown()` 单一路径：写 final 状态（`running=false` + `exit_at` /
`exit_ok`）→ exit 0。状态文件写失败 → `_error` 记录 + exit 1（不阻塞同步本身）。
SIGTERM 走 `tokio::signal::unix`（cfg unix）；Windows 仅 ctrl_c。

## 与其它 sync 的边界

| 术语 | 含义 | 触发方式 |
|---|---|---|
| `timeline sync` | 源 → timeline.db 事件拉取 | 显式命令 |
| `sync`（WebDAV） | 本地数据文件 ↔ WebDAV 双向文件同步 | 显式命令 / auto_sync |
| daemon sync cycle | 上述 timeline 拉取 + mail 全文件夹缓存 + rss 缓存 | 常驻进程周期自动 |
| `mail list` staleness | mail 缓存超 15 分钟自动同步 | 查询时隐式触发 |
