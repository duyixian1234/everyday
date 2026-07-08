# Progress Log — Everyday

## Session 2026-07-08 (启动)

### 已完成
- 勘察项目现状：空 Rust 项目（`everyday` 包名，仅 Hello World）
- 通读 PRD.md，确认范围：通讯（邮件/日历/RSS）+ 系统（监控/剪贴板）+ 网络（抓取/HTTP）+ 文件（搜索/解析）
- 创建规划三件套：`task_plan.md` / `findings.md` / `progress.md`
- 确认 Rust 工具链：`cargo 1.96.1` / `rustc 1.96.1`，edition 2024 可用

### 进行中
- Phase 1：项目地基与文档

### 下一步
- 更新 `Cargo.toml`（改名 + 依赖）
- 创建 `agents.md`
- 建立 `src/` 骨架

### 测试结果
_(暂无)_

### 错误记录
_(暂无)_

---

## Session 2026-07-08 (Phase 1-5 基础架构落地)

### 已完成
- **Phase 1 项目地基**：`Cargo.toml` 改名 `everyday` + 全量依赖；创建 `agents.md`（AI 协作规范）；建立 `src/` 骨架
- **Phase 2 多账户配置**：`config.rs` 实现 `Config` / `MailConfig` / `CalendarConfig` / `RssConfig`，多账户 + `default_account` + keyring 约定；`config.example.toml` 示例；`everyday config path/get/set/list/init` 子命令（点分路径读写 toml::Value）
- **Phase 3 核心抽象**：
  - `error.rs`：`AgentError` 枚举 + 自定义 Serialize → PRD 规定 JSON 格式 `{"error":"Type","message":"..."}`
  - `output.rs`：`Output` 枚举（Text/Json/Records）+ `RenderMode` + `finalize()` → 退出码
  - `modules/mod.rs`：`Executor` trait（async_trait）+ `ModuleRegistry` + `parse_simple_args()` 辅助
- **Phase 4 CLI**：`cli.rs` clap derive 扁平结构 `everyday <module> <action> [args]` + `--json`/`--account` 全局 flag；`main.rs` 分发 + `config` 特殊处理
- **Phase 5 模块骨架**：6 模块（email/calendar/rss/system/network/fs）实现 Executor；`system status` 已可工作（sysinfo，参考实现）

### 测试结果
- `cargo build` ✅ 通过（仅预留 API dead_code 警告，已加临时 `#![allow(dead_code)]`）
- `cargo clippy --all-targets` ✅ 零警告
- `cargo test` ✅ 25 passed / 0 failed
- 冒烟测试 ✅：
  - `everyday sys status` → 表格输出 CPU/内存/swap/磁盘
  - `everyday --json sys status` → 紧凑 JSON 数组
  - `everyday mail` → 模块帮助（actions 文档）
  - `everyday --json mail list`（无配置）→ `{"error":"AccountNotFound","message":"..."}` exit=1
  - `everyday bogus` → `{"error":"ModuleNotFound",...}` exit=1
  - `everyday config path` → Windows `%APPDATA%\everyday\config.toml`

### 错误记录（已解决）
- lettre `imap-pool` feature 不存在 → 改 `pool`+`tokio1-rustls-tls`+`builder`
- `format!("{s:<0$}",s,w)` 位置参数错位 → 改 `pad()` 自由函数
- sysinfo 0.30 `System::global_cpu_usage()`/`disks()` 不存在 → `sys.cpus()` 平均 + `Disks::new_with_refreshed_list()`
- `toml::Value::is_boolean` → `is_bool()`
- clippy `needless_range_loop` → zip 迭代

### 下一步（Phase 6）
- 实现 `fs` 模块（ignore/walkdir 内容搜索、目录树、read-json）
- 实现 `network` 模块（reqwest fetch→Markdown、通用 request）
- 补全 `system`（watch/clip）
- 邮件/日历/RSS 模块集成

---

## Session 2026-07-08 (Phase 6: 邮件模块完整实现)

### 已完成
- **`email` 模块完整实现**（IMAP 收件 + SMTP 发件 + keyring 凭证）：
  - `mail list [--unread] [--limit N] [--folder]` —— IMAP UID SEARCH + FETCH ENVELOPE 摘要
  - `mail read <uid>` —— FETCH BODY[] + mailparse 解析正文/headers
  - `mail search --query Q` —— IMAP SEARCH TEXT（转义双引号）
  - `mail send --to --subject --body [--cc]` —— lettre AsyncSmtpTransport（STARTTLS）
  - `mail login` —— rpassword 交互输入密码存系统 keyring（service=`everyday/mail/<account>`）
- **配置系统增强**：`config get/set` 支持数组索引（`mail.accounts.0.name`），set_dotted/get_dotted 增加 array 分支
- **依赖新增**：tokio-rustls/rustls(ring)/webpki-roots（IMAPS TLS）、mailparse（邮件解析）、rpassword（密码交互）、tokio-util compat（tokio↔futures AsyncRead 桥接）
- **默认 SMTP 端口** 465→587（STARTTLS 标准，lettre relay 兼容）

### 测试结果
- `cargo build` ✅、`cargo clippy --all-targets` ✅ 零警告、`cargo test` ✅ 33 passed（+8 邮件/数组测试）
- 冒烟测试 ✅：
  - `everyday mail` → 5 actions 帮助
  - `everyday config init` → 创建配置文件
  - `everyday config get mail.accounts.0.name` → `work`（数组索引读取）
  - `everyday config set mail.accounts.0.imap_port 993` → 数组索引写入
  - `everyday mail list --account work`（无密码）→ `AuthError` 提示 run `everyday mail login`
  - JSON 错误格式正确，退出码 1

### 错误记录（已解决）
- async-imap 基于 `futures` AsyncRead，tokio-rustls 是 tokio 的 → tokio-util compat `.compat()` 桥接
- `Fetch.envelope` 是方法非字段；`Envelope`/`Address` 来自 `imap_proto`，字段是 `Cow<[u8]>` → `from_utf8_lossy` 转
- `uid_search` 返回 `HashSet<u32>` 非 Stream → 直接 collect
- `mailparse::MailHeaderMap` 是 trait 不能作参数 → 改 `&ParsedMail`
- `lettre` `ContentType::TEXT_PLAIN_UTF_8` 不存在 → `TEXT_PLAIN`
- `config get/set` 不支持数组索引 → get_dotted/set_dotted 增加 array 分支

### 下一步
- `fs` 模块（ignore/walkdir）、`network` 模块（reqwest/scraper）、`system` 补全、`calendar`(CalDAV)、`rss`(feed-rs)

---

## Session 2026-07-08 (邮件模块增强：文件夹递归)

### 需求
用户用规则把不同来源邮件自动移动到不同文件夹，需要：
1. 列出邮箱文件夹目录
2. list/search 默认递归获取所有文件夹邮件
3. `--folder` 指定单文件夹

### 已完成
- 新增 `mail folders`：IMAP LIST 列出所有文件夹（过滤 `\NoSelect`）
- `mail list` / `mail search` 默认**递归所有文件夹**，输出加 `folder` 列标识来源
- `--folder NAME`：仅指定文件夹；`--no-recursive`：仅 INBOX
- 提取辅助函数：`list_all_folders` / `resolve_folders` / `collect_across_folders`
- 无法 SELECT 的文件夹（\NoSelect）自动跳过，单文件夹失败不致命

### 测试结果
- `cargo build`/`clippy`/`test`(33 passed) 全绿
- **真实连接验证**（用户邮箱）：
  - `everyday mail folders` → 列出 INBOX/Sent/Drafts/Junk + 25 个分类子文件夹（Github/12306/Cloudflare/Vercel/Google/Steam 等）
  - `everyday mail list --limit 5` → 递归输出，folder 列正确，中文 subject 解码正确
  - JSON 模式输出文件夹数组

### 错误记录（已解决）
- `Session::list` 两个参数都是 `Option<&str>`（不是 &str）→ `list(None, Some("*"))`

### 下一步
- `fs`/`network`/`system` 补全、`calendar`/`rss` 模块
