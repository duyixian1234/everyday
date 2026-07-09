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

---

## Session 2026-07-08 (邮件模块增强：IMAP UTF-7 中文解码)

### 需求
分类文件夹名（IMAP UTF-7 编码如 `&UXZO1mWHTvZZOQ-/Github&kBp35Q-`）需显示成可读中文。

### 已完成
- 新增 `decode_imap_utf7`：RFC 3501 §5.1.3 解码器（modified base64 + UTF-16BE），手写 base64 表（const fn，无依赖），用 `char` 迭代正确透传 UTF-8 中文
- `mail folders` / `mail list` / `mail search` 显示文件夹名时解码为中文
- `select_folder` 智能匹配：先直接 select（原始编码名/INBOX），失败再遍历所有文件夹匹配解码后的中文名 → `list/read/search --folder "其他文件夹/Github通知"` 可用中文
- `collect_across_folders` 用 `select_folder`，select 用原始名、显示用解码名

### 测试结果
- `cargo build`/`clippy`/`test`(40 passed) 全绿
- **真实邮箱验证**：
  - `mail folders` → `其他文件夹/QQ邮件订阅`、`其他文件夹/Github通知`、`其他文件夹/腾讯云通知`、`其他文件夹/微众银行账单`、`其他文件夹/微软通知` 等 25 个中文名
  - `mail list --folder "其他文件夹/Github通知" --limit 3` → 中文匹配成功，folder 列显示中文

### 错误记录（已解决）
- 首版 `decode_imap_utf7` 用 `bytes[i] as char` 破坏 UTF-8 中文输入 → 改 `char` 迭代透传
- `list --folder 中文` 返回空（`session.select(中文)` 失败被跳过）→ `collect_across_folders` 改用 `select_folder` 智能匹配

### 下一步
- `fs`/`network`/`system` 补全、`calendar`/`rss` 模块

---

## Session 2026-07-08 (修复递归 list 未展示其他文件夹)

### Bug
不加参数的 `mail list` 只显示 INBOX：`collect_across_folders` 在 INBOX 取够 `limit` 条就 `break`，不遍历其他文件夹。

### 修复
- 去掉 `break`，遍历所有文件夹，每个文件夹取最近 `limit` 条作为全局候选
- 排序改为按**邮件日期**降序（跨文件夹 UID 不连续）：`chrono::DateTime::parse_from_rfc2822` 解析 RFC 2822 日期，容错去掉括号注释如 "(UTC)"，有日期排前无日期排后
- 最后 truncate 到 limit

### 测试结果
- 42 单测全绿，clippy 零警告
- **真实邮箱验证**：`mail list --limit 15` 跨 7+ 文件夹按日期混合排序（Google通知/INBOX/Github通知/腾讯云通知/Cloudflare通知/Junk/Sent Messages/From Me），时区正确（13:45 GMT 排在 14:20 +0800 之前）

---

## Session 2026-07-08 (创建 everyday-cli agent skill)

### 需求
将 everyday 命令行的使用封装为 agent skill，便于 AI Agent 加载后正确调用 CLI；同时编写 README.md 供人类参考。

### 已完成
- 创建 `skills/everyday-cli/` 目录
- **SKILL.md**（agent skill 入口）：
  - frontmatter（summary + read_when 触发场景：邮件操作、系统监控、everyday 命令、JSON 交互等）
  - 命令结构 `everyday <module> <action> [options]`、全局 flag（--json/--account）、输出模式、错误格式
  - 7 个模块完整命令清单，逐条标注实现状态（✅ 已实现 / ⚠️ 骨架）：config/mail 完整可用、sys 部分可用、fs/net/cal/rss 为骨架
  - 配置说明、keyring 约定、典型工作流示例（首次配置邮件、AI 读取未读邮件、查看系统状态、发送邮件）、安全约定、实现状态总览
- **README.md**（人类可读文档）：
  - 项目简介、特性、安装步骤、快速开始
  - 完整命令参考（含每个模块的选项表）
  - Text/JSON 双输出模式示例（含真实输出样例）
  - 配置文件示例、凭证安全说明、多账户机制
  - 项目结构、技术栈、Executor trait 架构说明、实现状态表

### 产物
- `skills/everyday-cli/SKILL.md`
- `skills/everyday-cli/README.md`

### 下一步
- Phase 6 待实现模块：fs（ignore/walkdir）、net（reqwest/scraper）、sys 补全（watch/clip）、cal（CalDAV）、rss（feed-rs）
- 考虑将 skill 同步到 `~/.workbuddy/skills/` 以便跨项目使用

---

## Session 2026-07-09 (Phase 6: 日历模块完整实现)

### 需求
参考交接文档经验，重新实现被 reset 的 calendar 模块（CalDAV）。

### 已完成
- **`calendar` 模块完整实现**（CalDAV via libdav 0.10 + icalendar 0.17）：
  - `cal login` —— rpassword 交互输入密码存 keyring（校验空密码）
  - `cal calendars` —— principal→home-set→calendars 发现，列出日历集合（含 name/colour）
  - `cal list [--today|--date YYYY-MM-DD] [--limit N]` —— GetCalendarResources 全量拉取 + icalendar 解析 + 本地日期过滤
  - `cal add --title --start --end [--location --description --calendar]` —— icalendar 构造 VEVENT + PUT
  - `cal delete --id HREF` —— DELETE force
- **依赖新增**：libdav 0.10、icalendar 0.17(parser)、hyper 1、hyper-util 0.1、hyper-rustls 0.27(ring,webpki-tokio)、tower-http 0.6(auth)、http 1、http-body-util 0.1
- **main.rs** 入口统一 `rustls::crypto::ring::default_provider().install_default()`（解决 ring+aws-lc-rs feature unification panic）
- **build_client**：hyper+rustls(ring,webpki)+AddAuthorization Basic Auth + `find_context_path` well-known 探测（跳过 DNS SRV bootstrap）

### 关键决策
- 跳过 `bootstrap_via_service_discovery`（DNS SRV 国内不可用），用 `find_context_path` 只做 well-known 重定向
- QQ `/.well-known/caldav` 301 → `/calendar/`，`base_url` 是 pub 字段直接覆盖
- QQ 不支持 current-user-principal（404）→ 降级用 base_url 作 home set
- cal list 用 GetCalendarResources 全量+本地过滤（比服务端 time-range REPORT 可靠）
- 亲自读 libdav/icalendar 源码验证 API，修正交接文档二手信息（icalendar 输出已是 CRLF、DatePerhapsTime::date_naive 内置）

### 测试结果
- `cargo build` ✅、`cargo clippy --all-targets -- -D warnings` ✅ 零警告、`cargo test` ✅ 54 passed（+12 calendar 测试）
- **真实端到端验证**（QQ dav.qq.com）：
  - `cal calendars` → 4 个日历集合（含 "duyixian1234's QQMail Calendars"）
  - `cal add` → 事件创建成功（href + etag）
  - `cal list` → 正确拉取并解析（start/end/summary）
  - `cal delete` → 删除成功，list 恢复空
  - `cal bogus` → UnknownAction，错误 JSON 格式正确

### 错误记录（已解决）
- `http::Uri` 是 `host()` 非 `host_str()` → 改 `base.host()`
- `base` 被 `host` 借用后 move → `host` 转 owned String
- QQ current-user-principal 404 → 降级 base_url 作 home set

### 下一步
- `fs`/`net`/`sys` 补全、`rss` 模块

---

## Session 2026-07-09 (CLI 子命令帮助修复)

### Bug
`everyday cal add --help` 显示顶层 clap 帮助而非 `cal add` 的 action 帮助。
原因：clap 内置 `--help` flag 在顶层拦截，`trailing_var_arg + allow_hyphen_values` 无法阻止 clap 把 `--help` 当作自身帮助 flag。

### 修复方案
在 `Cli::parse()` 前预扫描 `std::env::args()`，检测出现在 module 之后的 `--help`/`-h`：
- `--help` 在 module 之前 → 返回 None，交给 clap 处理顶层帮助
- `--help` 在 module 之后、action 之前 → 输出 module 帮助
- `--help` 在 action 之后 → 输出 action 帮助（从 ActionDoc.usage 渲染）

新增函数：
- `detect_subcommand_help`：预扫描 raw args，正确跳过 `--json`、`--account <value>`、`--account=value`、`--key=value`
- `action_help`：从 ModuleRegistry 查找 ActionDoc，渲染单 action 详细帮助
- `config_help`：config 模块不在 registry 中，单独处理（module + action 两级）
- `render_help_target`：统一渲染帮助目标为 (exit_code, text)

### 测试结果
- `cargo build` ✅、`cargo clippy --all-targets -- -D warnings` ✅ 零警告、`cargo test` ✅ 69 passed（+15 新测试）
- 冒烟测试 ✅：
  - `everyday cal add --help` → action 帮助（--title/--start/--end 用法）
  - `everyday cal --help` / `everyday cal -h` → module 帮助（列出所有 actions）
  - `everyday --help` → 顶层 clap 帮助（不变）
  - `everyday mail send --help` / `everyday config set --help` / `everyday sys status --help` → 各 action 帮助
  - `everyday cal add --help --title foo` → help 优先（忽略后续 args）
  - `everyday --json cal add --help` → 全局 flag + 子命令帮助
  - `everyday sys status` → 正常输出（无回归）
  - `everyday cal bogus` → UnknownAction 错误（无回归）

### 下一步
- `fs`/`net`/`sys` 补全、`rss` 模块
