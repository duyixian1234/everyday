# agents.md — Everyday AI Agent 协作规范

> 本文件是给 AI Agent（以及人类协作者）的项目工作指南。在任何代码改动前，请先读完本文件。

## 项目概览

**Everyday** 是一个 Rust 编写的本地 CLI 工具集，作为 AI Agent 的"数字双手"。统一命令结构 `everyday <module> <action> [options]`，支持 Text / JSON 双输出。范围与定位见下文「范围与定位」节。

- **语言：** Rust (edition 2024)
- **二进制名：** `everyday`（见 `Cargo.toml` 的 `[[bin]]` 段）
- **目标平台：** Windows / macOS / Linux
- **异步运行时：** `tokio`
- **规划文件：** `task_plan.md` / `findings.md` / `progress.md`（使用 planning-with-files 工作流）

## 范围与定位

Everyday 是 AI Agent 连接**外部世界**的统一接口，定位为"外部集成接口"，而非通用系统工具箱。

- **保留的模块**封装代理自身难以实现的外部协议 / 状态 / 凭证：`mail`（IMAP/SMTP + keyring）、`cal`（CalDAV）、`rss`（feed 解析 + 状态）、`note`/`todo`（Notion API + keyring）。
- **不内置**文件搜索、HTTP 请求、系统监控、剪贴板等"通用能力"——这些代理用 shell / `curl` / `fd` / `rg` 即可直接完成，CLI 包装无差异化价值。
- 据此，`fs`、`net` 与 `sys` 模块均已移除（详见 `findings.md`）。

## 目录结构

```text
.
├── agents.md               # 本文件
├── Justfile                # 开发流程管理（just）
├── Cargo.toml              # 依赖与包元数据
├── config.example.toml     # 配置示例
├── task_plan.md            # 开发计划与阶段跟踪
├── findings.md             # 调研与技术决策记录
├── progress.md             # 会话进度日志
└── src/
    ├── main.rs             # 入口：解析 → 分发 → 渲染
    ├── cli.rs              # clap 命令定义
    ├── config.rs           # 配置加载与多账户管理
    ├── error.rs            # 统一错误类型 AgentError
    ├── output.rs           # Output 结构体（Text/JSON 渲染）
    ├── notion_client.rs    # 底层共享 Notion 客户端（HTTP/限流/反序列化）
    └── modules/
        ├── mod.rs          # Executor trait + ModuleRegistry
        ├── email.rs        # 邮件（IMAP/SMTP）
        ├── calendar.rs     # 日历（CalDAV）
        ├── rss.rs          # RSS/Atom 订阅
        ├── note.rs         # 笔记与知识库（Notion API）
        └── todo.rs         # 待办任务（Notion API，基于 notion_client）
```

## 核心架构约定

### 1. 命令结构
所有命令遵循 `everyday <module> <action> [options]`：
- `module`：`mail` | `cal` | `rss` | `note` | `todo` | `config`
- `action`：由各模块自定义（如 `list`、`send`、`status`）
- 全局 flag：`--json`（切换 JSON 输出）、`--account <name>`（指定账户）

### 2. Executor Trait（核心抽象）
每个模块实现 `Executor` trait，主程序只通过 trait object 调度：
```rust
#[async_trait]
pub trait Executor: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    async fn execute(&self, action: &str, args: &ActionArgs) -> Result<Output, AgentError>;
    fn actions(&self) -> Vec<ActionDoc>;  // 用于 `everyday <module> --help`
}
```
**不要**在 `main.rs` 里写模块特定逻辑。新模块 = 新文件 + 注册一行。

### 3. 输出层（Output）
所有模块返回 `Output`，由主程序统一渲染：
```rust
pub enum Output {
    Text(String),
    Json(serde_json::Value),
    Table(tabled::Table),
}
```
- `--json` 模式：`Text` → 原样输出字符串，`Json` → 紧凑 JSON，`Table` → 序列化为 JSON 数组
- 默认 Text 模式：`Text` → 原样，`Json` → pretty-print，`Table` → 终端表格

### 4. 错误处理
- 所有 `Result` 用 `Result<T, AgentError>`
- `AgentError` 实现 `serde::Serialize`，JSON 模式下输出 `{"error":"ErrorType","message":"..."}`
- 退出码：成功 0，失败 1（特定错误类型可扩展非零码）
- **禁止** `unwrap()`/`expect()` 出现在非测试代码中，用 `?` + 上下文

### 5. 配置与多账户
- 配置路径：`~/.config/everyday/config.toml`（用 `dirs::config_dir()` 跨平台解析）
- 每个模块支持多个命名账户，顶层 `[default_account]` 指定默认账户名
- **密码绝不存配置文件**，走 `keyring`（service = `everyday/<module>/<account>`）
- `--account` 覆盖默认；未找到账户 → `AgentError::AccountNotFound`

## 编码规范

### 风格
- 遵循 `rustfmt` 默认格式 + `clippy` 无警告
- 公开类型加 `#[derive(Debug, Clone)]`；配置结构体加 `#[derive(Deserialize, Serialize)]`
- 文档注释 `///` 用于 public API，模块级 `//!` 用于文件顶部说明
- 异步函数用 `async fn`，trait 方法加 `#[async_trait]`

### 命名
- 模块文件：小写（`email.rs`，不用 `mail.rs` —— 模块名描述领域）
- CLI 命令别名：`mail`→邮件、`cal`→日历
- 结构体：`PascalCase`；函数/变量：`snake_case`；常量：`SCREAMING_SNAKE_CASE`

### 依赖
- 新增依赖前在 `findings.md` 记录理由
- 优先 `rustls-tls`，避免 OpenSSL 链
- 避免 `default-features = true` 带入无用特性，按需开启

## 开发工作流

### 开发命令（just）

项目用 [`just`](https://github.com/casey/just) 统一管理开发流程，底层仍是 cargo 命令。安装：`cargo install just`（或系统包管理器）。

| 命令 | 等价 cargo 命令 | 说明 |
| --- | --- | --- |
| `just format` | `cargo fmt` | 格式化全部代码 |
| `just check` | `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` | 格式检查 + lint（零警告） |
| `just test` | `cargo test` | 运行测试 |
| `just build` | `cargo build` | 构建 |
| `just ci` | `check` → `test` → `build` | 完整 CI 流程 |
| `just` | `just --list` | 列出所有可用命令 |

> 提交前统一用 `just ci` 跑一遍；日常开发中 `just format` 修正格式，`just check` 做静态检查。

> 跨平台终端：Justfile 顶部用 `set shell := ["bash", "-c"]`（Unix）与 `set windows-shell := ["powershell.exe", "-NoProfile", "-NoLogo", "-Command"]`（Windows），无需额外配置即可在两类平台运行 `just`。

### 改动前
1. 读 `task_plan.md` 确认当前 Phase
2. 读相关源文件，理解现有实现
3. 在 `task_plan.md` 把对应任务标 `in_progress`

### 改动中
- 每 2 次外部抓取/搜索后，立即写入 `findings.md`
- 遇到错误立刻记入 `task_plan.md` 的 Errors Encountered 表

### 改动后
1. `cargo build` 必须通过
2. `cargo clippy --all-targets -- -D warnings` 无警告（或 `just check`）
3. `cargo fmt --check` 无差异（提交前先 `cargo fmt` 统一格式）—— 对齐 CI 的 `rustfmt --check` 门槛，漏跑会导致 Format check 直接失败
4. 受影响模块的单测通过
4. 更新 `progress.md`（已完成 / 下一步）
5. 把 task 状态标 `completed`
6. **每完成一次完整任务必须 git 提交**（见下方提交规范）

### 提交规范（Conventional Commits）

**核心规则：每完成一次完整任务（一个功能、一个模块、或一个 Phase）就进行一次 git 提交。**

保持提交原子化——一个 commit 只对应一个完整、可独立理解的任务单元：
- ✅ 不要攒一大批改动才提交；完成一个完整任务就立刻提交
- ✅ 不要把多个不相关的任务塞进同一个 commit
- ✅ 每次提交后项目应处于可编译、可运行的稳定状态

"完整任务"的判定标准（满足其一即可视为完成一个任务单元）：
- 实现了一个完整功能（如 `mail login` 可用、`cal add` 可用、`todo init-db` 可用）
- 完成一个模块的全部骨架或核心动作
- 完成 `task_plan.md` 中的一个 Phase
- 一组紧密相关、不可分割的小改动（如修复一个 bug + 其测试）

```
feat(<module>): <简述>          # 新功能
fix(<module>): <简述>           # 修 bug
refactor: <简述>                # 重构
docs: <简述>                    # 文档
chore: <简述>                   # 依赖/构建
test(<module>): <简述>          # 测试
```
`<module>` 可选，如 `feat(email): 支持 IMAP IDLE`。

**提交前检查清单：**
- [ ] `cargo build` 通过
- [ ] `cargo clippy --all-targets -- -D warnings` 无警告（或 `just check`）
- [ ] `cargo fmt --check` 通过（或已 `cargo fmt` 统一格式）
- [ ] `cargo test` 通过
- [ ] `progress.md` 已记录本次工作
- [ ] commit message 符合 Conventional Commits 格式

## 测试要求
- 配置加载、output 渲染、error 序列化必须有单测
- 每个模块的 Executor 至少有一个 happy-path 集成测试（用 `--json` 断言输出）
- 网络相关测试用 mock 或 `#[ignore]` 标注，CI 默认跳过
- 测试文件：单测 `#[cfg(test)] mod tests` 放源文件底部；集成测试 `tests/` 目录

## 安全红线
- ❌ 不得在配置文件、日志、输出中明文打印密码/token
- ❌ 不得 `unwrap()` 用户输入解析结果
- ✅ 网络请求必须设超时（`reqwest::Client::builder().timeout()`）
- ✅ 本地文件操作（如读取配置）需处理权限错误，返回 `AgentError::PermissionDenied`
- ✅ 凭证只通过 `keyring` 读写

## 性能预算
- 冷启动 < 100ms（避免在 main 早期做重 IO）
- 网络请求（RSS 抓取）支持异步流式（超时 + 并发）
- 大输出避免全量 buffer，必要时分块
