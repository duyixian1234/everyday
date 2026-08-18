## [0.17.4] - 2026-08-18

### 🚜 Refactor

- *(task)* Consolidate scheduler pass into a Scheduler context
- *(task)* Deepen task execution behind the module interface
- *(config)* Unify config writes on a comment-preserving ConfigEditor
- *(output)* Move exit code off Output onto RequestContext

### ⚙️ Miscellaneous Tasks

- Release v0.17.4
## [0.17.3] - 2026-08-18

### 🐛 Bug Fixes

- *(task)* Relay child output via OS handle on Windows consoles
- *(task)* Split stdout/stderr branches in Unix relay path

### 🧪 Testing

- *(daemon)* Hoist nested #[test] out of daemon_run_once_json_shape

### ⚙️ Miscellaneous Tasks

- Release v0.17.3
## [0.17.2] - 2026-08-18

### 🚀 Features

- *(task)* Add command execution and cron scheduling

### 🐛 Bug Fixes

- *(task)* CLI parse collision + scheduler/harness hardening

### 📚 Documentation

- Split README into docs/
- *(task)* 模块设计 ADR F017 + CONTEXT.md 术语（#17）

### ⚙️ Miscellaneous Tasks

- Remove stale temporary target ignore rules
- Release v0.17.2
## [0.17.1] - 2026-08-14

### 🚀 Features

- *(id)* Date-sequence IDs with PID segment — {prefix}{YYYYMMDD}-{pid:x}-{seq} (R021)

### ⚙️ Miscellaneous Tasks

- Release v0.17.1
## [0.17.0] - 2026-08-13

### 🚀 Features

- *(daemon)* [daemon] config section + CLI registration (t1, #11)
- *(daemon)* Sync cycle engine (t2, #12)
- *(daemon)* State file + status + anti-reentry (t3, #13)
- *(daemon)* File log + --once output shape + state-write error (t4, #14)
- *(daemon)* Graceful shutdown unified path + SIGTERM (t5, #15)

### 📚 Documentation

- *(daemon)* ADR F016 design + daemon ops guide (v0.17.0)
- *(daemon)* Align daemon.md with implementation + README/skill refs (t6, #16)

### 🧪 Testing

- *(daemon)* Fix child mutability in non-Windows integration tests (t3)

### ⚙️ Miscellaneous Tasks

- *(typos)* Allowlist /FO tasklist flag (daemon state pid probe)
- Release v0.17.0
## [0.16.2] - 2026-08-10

### 🐛 Bug Fixes

- *(auth)* Honor [auth] env_credentials on no-Config hot paths

### ⚙️ Miscellaneous Tasks

- Release v0.16.2
## [0.16.1] - 2026-08-10

### 🚀 Features

- *(cli)* Migrate warning sites to tracing, keep {"_warning"} shapes (#7)
- *(cli)* Migrate mcp serve logging to tracing, add serve quiet test (#8)

### 📚 Documentation

- *(cli)* Document default-quiet logging and -v/-vv semantics (#9)

### ⚙️ Miscellaneous Tasks

- Release v0.16.1
## [0.16.0] - 2026-08-10

### 🚀 Features

- *(cli)* Leveled logging via tracing — default quiet, -v/-vv opt-in (F015, #6)

### ⚙️ Miscellaneous Tasks

- Release v0.16.0
## [0.15.0] - 2026-08-09

### 🚀 Features

- *(mcp)* Expose everyday capabilities as an MCP server over stdio (F014)

### 🐛 Bug Fixes

- *(id)* Gen_id unique across processes via PID segment
- *(id)* Relax gen_id_embeds_pid test to not pin the seq suffix

### ⚙️ Miscellaneous Tasks

- Release v0.15.0
## [0.14.0] - 2026-08-09

### 🚀 Features

- *(auth)* Opt-in env-credential fallback when keyring is unavailable (R020)

### ⚙️ Miscellaneous Tasks

- Release v0.14.0
## [0.13.1] - 2026-08-09

### 🐛 Bug Fixes

- *(sync)* Per-invocation tmp dir to survive parallel runs
- *(ci)* Nextest 0.9 junit output moved to config file

### 📚 Documentation

- Document sync module in README and skills (v0.13.0)
- Record G001 quality tool suite ADR

### 🎨 Styling

- Rustfmt cli_contract.rs

### 🧪 Testing

- *(cli)* Lock CLI contract — commands, actions, config shape

### ⚙️ Miscellaneous Tasks

- *(ci)* Use nextest for CI tests with junit reports
- *(ci)* Add typos spelling gate
- *(release)* Add git-cliff changelog generation
- *(release)* Adopt cargo-dist for the release pipeline (G001 batch 2)
- Release v0.13.1
## [0.13.0] - 2026-08-09

### 🚀 Features

- *(sync)* Implement WebDAV device sync (D001-D003)
- *(cli)* Omit the action on single-action modules

### 🐛 Bug Fixes

- *(sync)* Converge state on real WebDAV (ETag-less PUT + hash)

### 🚜 Refactor

- *(modules)* Remove Notion provider — note/todo/bookmark local-only (v0.13)

### 📚 Documentation

- *(skills)* Split everyday-cli SKILL.md into references
- Add MIT LICENSE and agent-first narrative for GOAI competition
- Record R019 (remove Notion provider) in progress ADR timeline
- WebDAV device sync design — ADRs D001-D003, glossary, config example

### ⚙️ Miscellaneous Tasks

- Release v0.13.0
## [0.12.0] - 2026-08-05

### 🚜 Refactor

- *(core)* [**breaking**] Pass RequestContext explicitly through Executor (v0.12)

### ⚙️ Miscellaneous Tasks

- Release v0.12.0
## [0.11.0-rc] - 2026-08-05

### 🚀 Features

- *(docs)* Add domain documentation and issue tracker guidelines
- *(output)* Add TypedValue records preserving JSON types (P6)
- *(config)* Validate semantic config errors at load time (P2c)
- F012 Phase 3 — lifecycle hooks, request context, middleware stack

### 🚜 Refactor

- *(config)* Unify account resolution via AccountProvider trait (P2a)
- Address F012 Phase 1 code-review findings
- *(mail)* P1 service layer — MailBackend trait + macro ArgSpec (F012 Phase 2)
- *(modules)* Migrate all ArgSpec to cli_action!/flag! macros (P1, F012)
- *(cal,rss)* P1 service layer — CalBackend/RssBackend traits (F012 Phase 2)
- *(config)* P2b — inject config subsets, not full Config (F012 Phase 2)
- Address F012 Phase 2 code-review findings
- Address F012 Phase 3 code-review findings
- Complete F012 P1 special-module service wiring

### 📚 Documentation

- *(governance)* Split into universal methodology + project conventions
- Generalize governance.md and decouple progress from ADR index
- *(adr)* F012 architecture deepening phase (P1-P5 roadmap)
- *(adr)* Mark F012 Phase 1 implemented (P6/P2c/P2a)
- *(skills)* Sync mail list typed output (uid number, unread column)
- Mark F012 Phase 2 implemented (P1+P2b); update task_plan/progress
- Update F012 P3 status to 8/11 health_check overrides
- Record F012 P1 special-module service wiring completion

### ⚙️ Miscellaneous Tasks

- *(docs)* Consolidate link checking
- Release v0.11.0-rc
## [0.10.0] - 2026-07-14

### 🚀 Features

- *(memory)* Add memory module — append-only triple notebook (Phase 15 / K001-K004)

### 📚 Documentation

- Delete findings.md and scrub all references
- *(governance)* Add governance methodology document
- Strip execution traces from progress.md and task_plan.md per governance §4
- *(governance)* Decouple from project specifics and add tooling templates

### ⚙️ Miscellaneous Tasks

- Release v0.10.0 — memory module (Phase 15 / K001-K004)
## [0.9.0] - 2026-07-14

### 🚀 Features

- *(mail)* Add search_envelopes free-text query to envelope cache
- *(search)* Add mail module to unified search via envelope cache

### 📚 Documentation

- *(phase14)* ADR S007 + progress + version bump to v0.9.0

### ⚙️ Miscellaneous Tasks

- *(justfile)* Add -q to test/build for quieter output
## [0.8.1] - 2026-07-12

### 🐛 Bug Fixes

- *(rss)* UTF-8-safe summary truncation to stop char-boundary panic

### 🚜 Refactor

- *(note,todo,bookmark)* 目录脚手架 git mv + use 路径修正 (Phase 13 T13.1)
- *(note)* 引入 NoteBackend trait + 双实现 + 工厂切换 (Phase 13 T13.2)
- *(todo)* 引入 TodoBackend trait + 双实现 + 工厂切换 (Phase 13 T13.5)
- *(bookmark)* 引入 BookmarkBackend trait + 双实现 + 工厂切换 (Phase 13 T13.8)

### 📚 Documentation

- Add Phase 13 Backend DI plan (T13.1-T13.10) + ADRs R016-R018
- *(phase13)* 回填 Phase 13 完成小结 + 更新进度文档 (T13.10)

### 🧪 Testing

- *(note)* 加 MockNoteBackend + 动作层 DI 验收单测 (Phase 13 T13.3)
- *(todo)* 加 MockTodoBackend + 动作层 DI 验收单测 (Phase 13 T13.6)
- *(bookmark)* 加 MockBookmarkBackend + 动作层 DI 验收单测 (Phase 13 T13.9)

### ⚙️ Miscellaneous Tasks

- Release v0.8.1
## [0.8.0] - 2026-07-12

### 🚀 Features

- *(auth)* [**breaking**] Consolidate credential/login into top-level auth module

### 📚 Documentation

- *(auth)* Design
- *(auth)* Update references and mark Phase 12 complete

### ⚙️ Miscellaneous Tasks

- Release v0.8.0
## [0.7.0] - 2026-07-12

### 🚀 Features

- *(search)* Searchable trait + Hit/SearchQuery/SearchRegistry core
- *(note)* Searchable impl for note (local SQLite GLOB)
- *(todo)* Searchable impl for todo (local SQLite GLOB)
- *(bookmark)* Searchable impl for bookmark (local SQLite GLOB)
- *(rss)* Local item cache table + Searchable impl
- *(cal)* Searchable impl for calendar (full-pull + in-memory GLOB)
- *(search)* Wire up SearchModule + register in ModuleRegistry

### 🐛 Bug Fixes

- *(ci)* Force LF for shell scripts via .gitattributes

### 📚 Documentation

- *(plan)* Mark Phase 11 in-planning with ADR S001-S006
- Mark Phase 11 complete + reflect v0.7.0 across project docs

### ⚙️ Miscellaneous Tasks

- Release v0.7.0
## [0.6.2] - 2026-07-12

### 🐛 Bug Fixes

- *(docs)* Indent doc-list continuations to satisfy clippy 1.97 lints

### 🚜 Refactor

- *(comments)* Clean up comments across src/ and document policy

### 📚 Documentation

- *(adr)* Re-organize ADR index by module with prefix-numbered scheme
- Restructure agents.md / findings.md / progress.md around .rules/ + ADR catalog

### ⚙️ Miscellaneous Tasks

- *(justfile)* Translate comments to English
- Release v0.6.2
## [0.6.1] - 2026-07-11

### 🐛 Bug Fixes

- *(mail)* PoolGuard::session returns Result instead of panicking
- *(mail)* PoolGuard Drop no longer panics when tokio runtime is down
- *(timeline)* Eliminate double-unwrap on DST-boundary date parsing
- *(timeline)* CalProvider::sync honors the window argument
- *(output)* JSON serialize failure no longer breaks --json contract
- *(util)* Is_json() no longer scans std::env::args() (process pollution)
- *(args)* Parse_simple_args no longer misclassifies negative numbers
- *(ops-log)* Surface write failures to user (was let _ = silently)
- *(timeline)* Surface sync and DB write failures (were let _ = ...)
- *(mail)* Flags unread filter uses GLOB (not LIKE) to anchor token match
- *(timeline)* Reject unknown --source and invalid --limit explicitly
- *(timeline)* 修复 --from 单独给定被静默回退 preset 的问题

### 🚜 Refactor

- *(config)* Collapse 5 X_account() lookups into a single macro
- Collapse 3 KEYRING_USER const definitions into shared::keyring_user
- *(util)* Consolidate parse_rfc3339 + drop silent Utc::now() fallback
- *(mail)* Collapse select_folder_mailbox into select_folder_inner
- *(bookmark)* Collapse 2 parse_tags impls into local::parse_tags
- *(todo/bookmark)* Collapse 2 set_*_database_id into shared helper
- *(note/todo/bookmark)* Share notion login via local::login_notion
- *(mail)* Consolidate envelope field extraction into one helper
- *(timeline)* Consolidate build_providers todo/note/bookmark branch
- *(config)* Collapse TodoAccount/BookmarkAccount into NotionLocalAccount
- Extract ConfigModule as a real Executor
- *(cli)* Clap 子命令树 + 移除 help-registry 重建

### 📚 Documentation

- *(progress)* Record 7 review fixes from 2026-07-11 caveman review
- *(progress)* Expand review fix table to 11/26 with remaining backlog
- *(progress)* Mark 2026-07-11/12 review-backlog as 23/26 complete
- *(progress)* 记录 clap 子命令化重构完成 + 测试数校正

### 🧪 Testing

- *(timeline)* Fill in empty group_by_source_separates_sources test

### ⚙️ Miscellaneous Tasks

- *(fmt)* 规范化此前 review 留下的未提交文件格式
- Release v0.6.1
## [0.6.0] - 2026-07-11

### 🚀 Features

- *(mail)* Local envelope cache + connection pool sync

### ⚙️ Miscellaneous Tasks

- Release v0.6.0
## [0.5.0] - 2026-07-11

### 🚀 Features

- *(timeline)* Unified event log with append-only log + per-provider sync
- *(todo)* Add delete action for notion + local providers

### 🐛 Bug Fixes

- *(timeline)* Add OpsLogProvider so notion writes surface in timeline
- *(ops-log)* Capture text-mode writes too (default CLI mode)
- *(timeline)* Honor --since in query path (with sub-day precision)

### 📚 Documentation

- 精简文档
- *(timeline)* 更新 phase 9 / current state / 完成 ADR
- *(timeline)* 记录 4 处修补 ADR (045afa6/9a3ef49/8de8f26/32f67c1)

### ⚙️ Miscellaneous Tasks

- Release v0.5.0
## [0.4.0] - 2026-07-11

### 🚀 Features

- *(bookmark)* Add bookmark module with local sqlite and notion providers

### 🐛 Bug Fixes

- *(just)* 设置 shell=bash(Unix) 与 windows-shell(Windows) 避免 sh 缺失
- *(just)* Check 拆为依赖 recipe 实现严格 fail-fast

### 🚜 Refactor

- 分层 modules/shared/util，去重 gen_id 与 json 探测

### 📚 Documentation

- Rewrite README and skills/README in English, keep Chinese as README_ZH
- *(agents)* 验收规则加入 cargo fmt 门槛，对齐 CI rustfmt --check
- *(bookmark)* Document bookmark module in config example, README, and skills

### 🎨 Styling

- 运行 cargo fmt 修复 CI 格式检查\n\nCI 仅因 rustfmt --check 失败；本地此前未跑 fmt。\n补 fmt（含 notion_client 及 note/todo 模块存量格式）。

### ⚙️ Miscellaneous Tasks

- Add Justfile and document dev workflow
- Release v0.4.0
## [0.3.0] - 2026-07-10

### 🚀 Features

- *(note,todo)* 新增本地 SQLite provider（sqlx）
- *(note,todo)* 默认 provider 改为 local

### 🚜 Refactor

- 移除 dead_code 抑制并将 note 模块接入共享 notion-client

### ⚙️ Miscellaneous Tasks

- Release v0.3.0
## [0.2.0] - 2026-07-10

### 🚀 Features

- *(todo)* Add Notion task database module with shared notion-client SDK

### 🐛 Bug Fixes

- *(todo)* 统一 Status 属性为 select 类型修复 Notion 过滤器类型不匹配

### 📚 Documentation

- Document macOS Apple Silicon (aarch64) prebuilt binaries
- 移除过时的 PRD.md
- 精简 progress/findings/task_plan，压缩历史进度保留核心 ADR

### ⚙️ Miscellaneous Tasks

- Add aarch64-apple-darwin (Apple Silicon) to CI and release matrices
- Release v0.2.0
## [0.1.0] - 2026-07-10

### 🚀 Features

- 实现核心架构（配置/错误/输出/Executor trait + CLI + 模块骨架）
- *(email)* 完整实现邮件模块（IMAP 收件/SMTP 发件/keyring 凭证）
- *(email)* 支持文件夹列表与递归读取所有文件夹邮件
- *(email)* 文件夹名 IMAP UTF-7 解码显示中文 + 智能匹配
- *(calendar)* CalDAV module via libdav + icalendar
- *(calendar)* 中文化日历列表 + list 默认今天及未来日程
- *(calendar)* 配置文件支持忽略日历
- *(rss)* 完整实现 RSS 模块，支持订阅源管理和抓取聚合
- *(note)* 基于 Notion API 的笔记/知识库模块
- *(note)* 新增 list 子命令列出指定数据库下的页面

### 🐛 Bug Fixes

- *(email)* 递归 list 遍历所有文件夹并按日期全局排序
- --account 全局 flag 注入 args 传给模块
- *(cli)* 子命令 --help 显示对应 module/action 帮助
- *(email)* 更新邮件读取命令文档，默认递归查找所有文件夹

### 🚜 Refactor

- *(calendar)* Ignore_calendars 改为账户级配置
- 移除 fs/net/sys 模块，定位收窄为纯外部集成接口

### 📚 Documentation

- 添加 agents.md 协作规范与开发计划跟踪
- 添加 everyday-cli agent skill 与项目 README
- 在 README 与 skills 文档中加入 note 模块说明
- Add download & install instructions for prebuilt binaries

### 🎨 Styling

- Apply rustfmt and fix clippy lints so CI passes

### ⚙️ Miscellaneous Tasks

- 初始化项目结构与依赖
- Add GitHub Actions workflow for master push (linux/macos/windows)
- *(release)* Add release workflow building 3-platform binaries
