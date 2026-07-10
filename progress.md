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

---
## Session 2026-07-09 (Phase 6: RSS 模块完整实现)

### 需求
补全被留作骨架的 `rss` 模块（feed-rs 2.4 解析）。

### 已完成
- **`rss` 模块完整实现**（reqwest 抓取 + feed-rs 解析）：
  - `rss follow --name N --url URL [--category C]` —— 写入 `[[rss.feeds]]`，toml::Value 局部编辑（只动 `rss.feeds`，保留 mail/cal 账户、不重排）
  - `rss list` —— 列出订阅源（name/url/category）
  - `rss unfollow --name N` —— 删除订阅源
  - `rss digest [--limit N] [--name FEED] [--category C]` —— 并发抓取、feed-rs 解析、按发布时间降序聚合
  - `rss fetch --name N [--limit N]` —— 抓取单个源列条目
- **网络**：`reqwest` 带 20s 超时 + UA（`everyday/<version>`），复用 main.rs 安装的 ring provider
- **最佳努力**：单源抓取/解析失败不致命，全部失败才报错（与 cal list 单日历降级一致）
- **附带修复（CLI 框架）**：`--json` 出现在模块动作之后的 trailing args 中时，clap 的 `trailing_var_arg` 会把它吞进模块 args 而非识别为全局 flag。在 `main.rs` 预扫描 raw args 补一道 OR 检测，确保 `--json` 任何位置都生效（AI Agent 交互主模式，丢失会静默退回文本）

### 测试结果
- `cargo build` ✅、`cargo clippy --all-targets -- -D warnings` ✅ 零警告、`cargo test` ✅ 86 passed（+6 rss 单测，含真实 Atom 样例解析）
- **真实端到端验证**（Hacker News RSS）：
  - `rss follow` / `list` / `unfollow` 循环正确，重复 follow 报错、非法 URL 报错
  - `rss fetch --name hn` → 解析出标题/链接/作者/时间
  - `rss digest` → 跨源聚合、按时间降序、limit 截断正确
  - `rss digest --name bogus` → InvalidArgument；`rss bogus` → UnknownAction
  - `--json` 在 `--flag value` 之后仍正确输出 JSON 数组（修复验证）
- 验证后已用备份还原用户原始 `config.toml`（仅保留原 `hackernews` 源）

### 错误记录（已解决）
- `toml::Value` 索引 `root["mail"] = ...` 不自动插入、缺失键直接 panic "index not found" → 改用 `.as_table_mut().unwrap().insert(...)`
- RSS2.0 `<author>` 语义是 email，feed-rs 不会把纯文本作者名解析进 `entry.authors` → 测试夹具改用 Atom（`<author><name>Bob</name>` 语义清晰）
- `Utc::with_ymd_and_hms` 需 `chrono::TimeZone` trait 在作用域 → 测试模块加 `use chrono::TimeZone;`
- clippy `collapsible_if` → `if let ... && cond` 折叠
- `--json` 位置被吞 → main.rs 预扫描 raw args OR 检测

### 下一步
- `fs`/`net`/`sys` 补全；Phase 7 全量构建/测试/文档

---

## Session 2026-07-10 (范围变更：移除 fs/net 模块，收窄定位)

### 需求
用户评审认为 `fs` / `net` 模块封装的是代理可用 shell / `curl` / `fd` / `rg` 直接完成的通用能力，过于底层，不符合 everyday「外部集成接口」定位。决定收窄定位，移除 fs + net，仅保留 mail / cal / rss + sys 的感知类动作。

### 已完成
- 删除 `src/modules/fs.rs` 与 `src/modules/network.rs`，注销 `ModuleRegistry` 中两模块的声明与注册
- `cli.rs` 的 `long_about` 与 `module` 字段注释去掉 net/fs
- `sys` 模块收敛为 `status` + `watch`，移除 `clip`（剪贴板，非感知类动作）
- `Cargo.toml` 移除仅 fs/net 使用的 `scraper` / `ignore` / `walkdir` / `arboard`；保留 `reqwest`（rss 复用）与 `notify`（sys watch 预留）
- `agents.md`：新增「范围与定位」节，同步目录结构 / 命令结构 / 命名别名 / 性能·安全备注
- `task_plan.md`：Phase 6 移除 fs/net 待办、状态汇总更新、关键设计决策表补范围行
- `findings.md`：新增「架构决策：移除 fs / net 模块」记录理由与决策
- `README.md` / `skills/everyday-cli/SKILL.md` / `references/COMMANDS.md`：移除 fs/net 与 sys clip 引用

### 测试结果
- `cargo build` ✅、`cargo clippy --all-targets -- -D warnings` ✅ 零警告、`cargo test` ✅ 86 passed / 0 failed（与移除前持平，fs/net 无单测）
- 冒烟测试 ✅：
  - `everyday fs ...` / `everyday net ...` → `{"error":"ModuleNotFound",...}` exit=1
  - `everyday sys status --json` → 正常 JSON 数组（cpu/memory/disk）

### 下一步
- Phase 7 全量构建/测试/文档
- 视需要补全 `sys watch`（notify）

---

## Session 2026-07-10 (范围再收窄：移除 sys 模块)

### 需求
用户进一步评审：`sys` 模块（系统资源监控）整体不保留，定位收窄为纯外部集成接口（mail/cal/rss + config）。同时要求 SKILL.md 只正向介绍现有模块，不要以"移除了哪些模块"的口吻编写。

### 已完成
- 删除 `src/modules/system.rs`，注销 `ModuleRegistry` 中 `sys` 的声明与注册
- `cli.rs` 模块列表（`long_about` + `module` 字段）去掉 `sys`
- `Cargo.toml` 移除仅 sys 使用的 `sysinfo` 与 `notify`（sys watch 预留也不再需要）
- `agents.md`「范围与定位」改为仅外部集成（mail/cal/rss），不内置项补充"系统监控"；目录结构 / 命令结构 / 命名别名去 sys；提交规范示例改用现存模块
- `task_plan.md`：Phase 6 删 `sys watch` 待办、范围变更说明补 sys 移除、设计决策表模块范围行、Phase 状态汇总去 sys
- `findings.md`：架构决策节补 sys 整体移除理由、`notify` 不再预留
- `README.md` / `COMMANDS.md`：删 sys 段与实现状态行；并修正 cal/rss 实现状态为"已实现"（原文档误标"待实现/骨架"，与代码实际不符）
- `SKILL.md` 重写：只介绍现有模块（mail/cal/rss/config），去掉"Removed modules"表述与 system status 示例

### 测试结果
- `cargo build` ✅、`cargo clippy --all-targets -- -D warnings` ✅ 零警告、`cargo test` ✅ 84 passed（较移除前 86 少 2，即 system.rs 的 2 个单测）
- 冒烟测试 ✅：`everyday sys ...` → `{"error":"ModuleNotFound",...}` exit=1；`cal`/`rss`/`mail` 正常

### 下一步
- Phase 7 全量构建/测试/文档
- 历史文档口径：原 PRD.md 已移除，范围以 `agents.md`「范围与定位」节为权威说明

---

## Session 2026-07-10 (新增 note 模块：Notion 笔记/知识库)

### 需求
用户给出 note 模块初步设计：基于 Notion API，提供 `login`/`search`/`create`/`read`/`append`/`update` 六个动作；配置引入 `provider` 字段 + `default_database_id`/`default_page_id` 预设；凭证走 keyring；屏蔽 Block 嵌套，提供纯文本/Markdown 追加与简化属性操作；支持文本/JSON 双输出。

### 已完成
- `src/config.rs`：新增 `DefaultAccount.note`、`NoteConfig`、`NoteAccount { name, provider(default=notion), default_database_id?, default_page_id? }`、`Config::note_account()` 解析；补充 3 个单测。
- `src/modules/note.rs`（新）：实现六动作 + Notion HTTP 封装（请求/分页拉 block/递归聚合为 Markdown/文本→block 切分/属性精确编码/keyring 凭证）；12 个纯函数单测（参数解析、属性编码、rich_text、标题提取、block 渲染等）。
- `src/modules/mod.rs`：注册 `note` 模块。
- `src/cli.rs`：`long_about` 模块列表加入 note。
- `src/main.rs` 内 `example_config()` 与 `config.example.toml`：加入 note 账户示例。
- 复用既有 `reqwest`（json + rustls-tls），**未新增依赖**。

### 测试结果
- `cargo build` ✅、`cargo clippy -- -D warnings` ✅ 零警告、`cargo test` ✅ 100 passed（含 note 模块 12 个新单测）。
- 冒烟测试 ✅：`everyday note --help` / `everyday note read --help` 正常列出动作；无账户配置时 `note login` 文本与 JSON 错误路径均正确（AccountNotFound）。

### 下一步
- 真实 Notion 联调（需用户提供 Integration Token + 数据库/页面 ID，并授予 integration 访问权限）。
- 视反馈扩展更多 block 类型或 `provider`（obsidian/feishu）。

---

## Session 2026-07-10 (note 新增 list 子命令)

### 需求
用户要求给 note 模块加 `list` 子命令，列出指定数据库下的页面。

### 已完成
- `src/modules/note.rs`：新增 `note_list`（动作 `list`），通过 `POST /databases/{id}/query` 分页拉取页面，支持 `--db`（缺省取 `default_database_id`）与 `--limit`（默认 50，上限 100）；文本模式输出 `id/title/last_edited` 表格，JSON 模式返回含 `properties`（简化字符串）的对象数组。
- `actions()` 注册 `list`；`Executor::description` 更新；dispatch 增加 `list` 分支；模块文档补全。
- 未新增依赖，复用已有 HTTP/分页/属性提取逻辑。

### 测试结果
- `cargo build` ✅、`cargo clippy -- -D warnings` ✅ 零警告、`cargo test` ✅ 100 passed。
- 真实联调 ✅：环境中已存在 note 账户与 keyring token，`everyday note list`（默认库）、`--json`、`--limit N` 均返回正确数据（如 Quick Note 页面）。

### 下一步
- 视需要为 list 增加按属性过滤（`--filter` 对应 Notion query filter）或排序。

---

## Session 2026-07-10 (为 note 模块补文档)

### 需求
用户在 README 与 skills 文档中加入 note 工具的介绍与说明。

### 已完成
- `README.md`：命令参考新增 `note` 章节（七动作 + 选项说明 + Notion 前置步骤）；intro、配置示例、项目结构图、实现状态表、使用示例均补 note；配置示例加 `[[note.accounts]]`。
- `skills/everyday-cli/SKILL.md`：frontmatter 描述、模块列表、Rule #4、Common tasks 均加 note 任务（search/list/create/read/append/update + 首次 setup）。
- `skills/everyday-cli/references/COMMANDS.md`：实现状态表加 note；新增完整 note 章节（命令表/选项/六个动作的 JSON 输出 schema）；配置示例加 note 账户与 `default_account.note`。
- `skills/README.md`：纠正过时内容（原仍列 sys/fs/net 骨架），改为 config/mail/cal/rss/note ✅ 可用，intro 覆盖笔记（Notion）。
- 所有 JSON schema 均对照 `src/modules/note.rs` 实际输出核对，确保文档与代码一致。

### 测试结果
- 纯文档改动，无代码变更；未触发构建/测试。

---

## Session 2026-07-10 (新增 todo 模块：Notion 待办任务)

### 需求
参考用户给出的「Notion 客户端 + 待办模块」两层架构设计文档，实现共享 `notion-client` 基础设施与上层的 `todo` 业务模块（login / init-db / list / add / start / complete）。

### 已完成
- **`src/notion_client.rs`（新，底层共享 SDK）**：`NotionClient` 持有带 `Authorization`/`Notion-Version` 头的 reqwest 客户端；通用 `request<B,R>` + `get/post/patch`；内置 **429 退避重试一次**（读 `Retry-After`，缺省 1s），平滑 Notion 限流；不使用 `unwrap()`（构造失败收拢为 `AgentError`）。
- **`src/modules/todo.rs`（新，业务层）**：
  - 干净 DTO `TodoItem { id, title, status, due?, priority? }` + Notion 原始结构 `NotionPage`/`TodoProperties`（TitleProperty/StatusProperty/DateProperty/SelectProperty 等强类型叶子）+ `From<NotionPage> for TodoItem` 双向映射。
  - 动作：`login`（token 存 keyring，service=`everyday/todo/<account>`）、`init-db`（`POST /v1/databases` 创建 Task/Status/Due/Priority，回填 `database_id` 到 config 局部编辑）、`list`（`/query` 含 filter(≠Done)+sorts(Due asc)，客户端兜底，`--all` 不过滤）、`add`（设计未列但必需，`--title` 必填）、`start`/`complete`（`PATCH /pages/{id}` 改 Status）。
  - 双输出判别复用 `std::env::args().any(|a| a=="--json")`（与 note 一致）。
- **`src/config.rs`**：新增 `DefaultAccount.todo`、`TodoConfig`、`TodoAccount { name, provider(=notion), parent_page_id?, default_database_id? }`、`Config.todo`、`Config::todo_account()`；补 4 个单测。
- **`src/modules/mod.rs`**：注册 `todo` 模块 + `pub mod todo;`。
- **`src/main.rs`**：`mod notion_client;` + `example_config()` 加 todo 账户与 `default_account.todo`；**`config.example.toml`** 同步 todo 账户示例。
- **`src/cli.rs`**：`long_about` 与 `module` 注释模块列表加 `todo`。
- **`findings.md`**：新增 todo 实现记录，明确与官方 design 的**有意偏差**（不新增 `AgentError` 变体 / 禁 unwrap / 不引入 toml_edit / note 暂不迁移复用），理由见文档。

### 与设计的偏差（以当前项目状态为准）
- 设计建议新增 `AgentError::NotionApiError/CredentialsMissing/ConfigWriteError` → 复用现有 `Auth`/`Network`/`Config`/`Other`（note 已用同映射，避免分裂分类）。
- 设计 `NotionClient::new` 用 `.unwrap()` → 改为返回 `Result`（安全红线）。
- 设计用 `toml_edit` 写回 → 用既有 `toml` crate `toml::Value` 局部编辑（零新增依赖）。
- 设计称 notion-client 为 todo+note 共享 → note 已完整可用，本次仅 todo 接入，note 保留内联 HTTP（择机去重）。

### 测试结果
- `cargo build` ✅、`cargo clippy --all-targets -- -D warnings` ✅ 零警告、`cargo test` ✅ 112 passed（原 100 + todo 6 + notion_client 3 + config 4）。
- 冒烟 ✅：`everyday todo --help` 列出 6 动作；无配置 `todo list --json` → `{"error":"AccountNotFound",...}` 退出码 1。
- 真实 Notion 联调待用户提供服务端 Token + parent_page_id（沿用 note 已验证的 keyring/端点模式）。

### 下一步
- 真实 Notion 联调（提供 Integration Token + parent_page_id，并授予 integration 访问权限）。
- 视反馈为 `list` 增加 `--filter`/`--sort` 自定义，或将 note 迁移到共享 `notion_client` 去重。

---

## Session 2026-07-10 (移除过时的 PRD.md)

### 需求
用户指出 PRD.md 已过时（仍描述已移除的 `fs`/`net`/`sys`/剪贴板模块），要求移除。

### 已完成
- `git rm PRD.md`：删除过时文档（原本约定只读，但定位早已收窄为"外部集成接口"，文档与现实脱节）。
- 清理指向 PRD.md 的引用：
  - `agents.md`：概述去掉"详见 PRD.md"，改指「范围与定位」节；目录树删除 PRD.md 行。
  - `README.md`：目录树删除 PRD.md 行。
  - `task_plan.md`：头部 `**PRD：**` 改为指向 `agents.md`「范围与定位」节并标注已移除。
  - `progress.md` / `findings.md`：将"PRD.md 按约定只读"的历史记录更新为"已移除"。
  - `src/cli.rs` / `src/error.rs` / `src/output.rs`：注释与测试名中的 "PRD" 改为 "agents.md"，避免指向已删文档。

### 测试结果
- `cargo build` ✅、`cargo clippy --all-targets -- -D warnings` ✅ 零警告、`cargo test` ✅ 113 passed。
- 确认全仓不再有指向 `PRD.md` 文件路径的活跃引用（仅剩历史记录中的"已移除"说明）。

