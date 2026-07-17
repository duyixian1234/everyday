# 项目级约定(Everyday Conventions)

> 适用范围:仅 Everyday 本项目(Rust CLI 工具集)。
> 通用治理方法论见 [`governance.md`](./governance.md);本文档只放与本项目形态
> 强绑定的项目级条款。
>
> **读法**:先读 `governance.md` 通用方法论,再回到本文档看项目级补充。
> 两份文档并存,任何冲突以本文档为准(项目级 override 通用方法论)。

---

## 目录

1. [项目级补充原则](#1-项目级补充原则)
2. [开发工作流(项目层)](#2-开发工作流项目层)
3. [任务运行器(项目层)](#3-任务运行器项目层)
4. [跨文档引用(项目层)](#4-跨文档引用项目层)
5. [注释规范(项目层)](#5-注释规范项目层)
6. [测试规范(项目层)](#6-测试规范项目层)
7. [安全红线(项目层)](#7-安全红线项目层)
8. [编码风格(项目层)](#8-编码风格项目层)
9. [发版与发版流水(项目层)](#9-发版与发版流水项目层)
10. [新项目初始化清单(项目层)](#10-新项目初始化清单项目层)

---

## 1. 项目级补充原则

在 [`governance.md` §1 核心原则](./governance.md#1-核心原则) 之上,本项目补充:

| 原则 | 含义 |
| --- | --- |
| **CLI 双输出契约** | 每个模块的每个 action 必须同时支持 Text 与结构化(JSON)输出;AI 交互走 JSON,人类交互走 Text。 |
| **append-only 优先** | 跨模块事件层(Timeline)与凭据/事实层(Memory)均为 append-only,禁止在主路径上做就地更新。 |
| **多账户一等公民** | 每个外部协议模块(mail/cal/note/todo/bookmark)必须支持多账户,凭证命名空间固定 `<项目>:<模块>:<账户>`。 |
| **Rust 工具链锁定** | Rust edition 2024;`Cargo.lock` 必须入库;`cargo clippy -- -D warnings` 零警告门禁。 |

---

## 2. 开发工作流(项目层)

补充 [`governance.md` §6](./governance.md#6-开发工作流) 的项目级细化。

### 2.1 不提交半成品

- Rust `Result` 链路上有未处理的 `Err`/`?` 时,`cargo build` 与 `cargo test` 无法通过;`cargo clippy -- -D warnings` 会捕获所有 `unwrap`/`expect`/未使用导入。
- 提交前必须保证 `just ci` 全绿,不可借 `#[ignore]` 跳过未通过的测试。

### 2.2 任务完成的项目级五步

| # | 步骤 | 命令 | 通过条件 |
| --- | --- | --- | --- |
| 1 | 质量门禁 | `just ci` | format / clippy / test / build 全绿 |
| 2 | 文档链接完整性 | `just check-links` | 无 FAIL |
| 3 | ADR 抽取 | 见 `governance.md` §7 | 决策性内容均落 ADR |
| 4 | 提交 | 见 `governance.md` §8 | 提交消息符合 Conventional Commits |
| 5 | 进度文档更新 | 在 `progress.md` 时间序索引追加新 ADR id | 索引行数 = 实际 ADR 数 |

> 门禁顺序按 Rust 工具链特性固定:`format → lint → test → build`。
> `format` 必须在 `lint` 前跑——格式不对时 lint 的提示往往误导。

---

## 3. 任务运行器(项目层)

### 3.1 选用 `just`

本项目实际选用 `just` 作为任务运行器,配置见 [`Justfile`](./Justfile)。

### 3.2 Justfile 最小可工作模板

```just
# Cross-platform shells: bash on Unix, PowerShell on Windows.
set shell := ["bash", "-c"]
set windows-shell := ["powershell.exe", "-NoProfile", "-NoLogo", "-Command"]

# List available recipes (also the default target).
default:
    @just --list

# Auto-format all sources.
format:
    cargo fmt

# Format + lint; fail-fast on format before running the linter.
check: _fmt-check _lint

_fmt-check:
    cargo fmt --check

_lint:
    cargo clippy --all-targets -- -D warnings

# Run tests, quiet.
test:
    cargo test -q

# Build, quiet.
build:
    cargo build -q

# Cross-document link integrity.
check-links:
    bash scripts/check-doc-links.sh

# Full local CI: check -> check-links -> test -> build.
ci: check check-links test build
```

### 3.3 工具替换表(Rust 专版)

| 占位 | Rust |
| --- | --- |
| `<formatter> --check` | `cargo fmt --check` |
| `<linter> --deny warnings` | `cargo clippy --all-targets -- -D warnings` |
| `<test-runner> -q` | `cargo test -q` |
| `<builder> -q` | `cargo build -q` |

> 其他语言项目的等价替换表见 [`governance.md` §10.6](./governance.md#106-精简-justfile-模板)。

### 3.4 `check-links` bash 脚本(项目版)

排除路径固化 Rust 项目形态(补充 `governance.md` §10.7 的通用骨架):

```bash
#!/usr/bin/env bash
# scripts/check-doc-links.sh — minimal cross-document link checker.
set -u
ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$ROOT" || exit 2

FAILS=0
mapfile -t FILES < <(find . -type f -name '*.md' \
  -not -path './target/*' -not -path './.git/*' \
  -not -path './node_modules/*' -not -path './dist/*' \
  -not -path '*/.workbuddy/*')

for f in "${FILES[@]}"; do
  dir="$(dirname "$f")"
  # Strip fenced code blocks, then extract every [label](target).
  awk 'BEGIN{c=0} /^```/{c=!c; next} !c' "$f" \
    | grep -oE '\[[^]]*\]\([^)]+\)' \
    | sed -E 's/.*\]\(([^)]+)\).*/\1/' \
    | while IFS= read -r target; do
        # Skip external / pure anchor.
        [[ "$target" =~ ^(https?:|mailto:|#) ]] && continue
        # Drop fragment.
        path="${target%%#*}"
        [[ -z "$path" ]] && continue
        # Skip obvious placeholders.
        [[ "$path" =~ [\<\>] ]] && continue
        # Resolve relative to the file's directory.
        resolved="$(cd "$dir" 2>/dev/null && realpath -m --relative-to=. "$path" 2>/dev/null || echo "$dir/$path")"
        if [[ ! -e "$resolved" ]]; then
          printf '[FAIL] %s: %s -> %s\n' "$f" "$target" "$resolved"
          FAILS=$((FAILS + 1))
        fi
      done
done

[[ $FAILS -eq 0 ]] && echo "[OK] no broken links across ${#FILES[@]} files." && exit 0
echo "[FAIL] $FAILS broken link(s)." && exit 1
```

> 要点(项目级补充):
> - 排除 `./target/*`(Rust 构建产物);若引入 frontend 子项目,再追加 `./.next/*` / `./out/*`。
> - 链接相对源文件目录解析,ADR 根目录(`docs/adr/`)的绝对位置无关。

---

## 4. 跨文档引用(项目层)

补充 [`governance.md` §11](./governance.md#11-跨文档引用与链接完整性) 的项目级细则。

### 4.1 链接深度表(本项目实际形态)

本项目是 Rust crate 平铺 + 一层 `src/` 模块,源文件目录层级典型 ≤ 2 层:

| 源文件位置 | ADR 链接前缀 |
| --- | --- |
| 顶层(如 `Justfile`、`agents.md`、`governance.md`) | `[id](docs/adr/<id>-...md)` |
| 一层子目录(如 `src/<module>/`) | `[id](../docs/adr/<id>-...md)` |
| 两层子目录(如 `src/<module>/<sub>/`) | `[id](../../docs/adr/<id>-...md)` |

> 若新成员误判层级,`just check-links` 会立刻报错。

### 4.2 源码注释里的跨 ADR 引用

Rust 源码中的 ADR 引用写在 `///` / `//!` / `/** */` / 普通 `//` 注释里,`check-links`
脚本通过 `awk '!c'`(剔除 fenced code)抽取后解析;`///` 等内嵌 markdown 不被 IDE
重构插件覆盖,改名时必须手动 grep。

---

## 5. 注释规范(项目层)

整节项目级化(见 `governance.md` §12 通用原则,本节给出项目级具体桶分类)。

### 5.1 四桶分类(Rust 项目)

任何包含非通用语的行,先判定它是"注释"还是"字符串字面量":

| 桶 | 类别 | 处理 |
| --- | --- | --- |
| 1 | 注释含非通用语(`//`、`///`、`//!`、`/** */`) | 翻译为通用语;若是决策内容,替换为 ADR 链接 |
| 2 | 章节横幅注释(如 `// === Section ===`) | 翻译为通用语,保留 `// ===` 风格 |
| 3 | 用户可见字符串(CLI 帮助文案、日志 message、错误信封内容) | **保留**——面向终端用户 |
| 4 | 测试 fixture / 示例数据 / 业务 URL 字面量 | **保留**——不是注释 |

> 判定疑问句:这一行是注释吗?是 → 走翻译;否 → 保留。

### 5.2 决策注释的处理

```rust
// Pulled from current-state snapshot, not full history.
// See [R007](../../docs/adr/R007-snapshot-vs-history.md).
```

判定疑问句:

- 是否说明未来代码必须遵守的约束?→ ADR
- 是否说明为什么选这种形态?→ ADR
- 是否只是重复代码本身?→ 删掉,代码已经说清楚了

### 5.3 注释清理的提交纪律

- 每个被清理的源文件 = 一个独立 commit
- 不允许把两个文件的注释翻译打包成一次提交(diff 难审、难回退)
- commit 类型用 `refactor(comments)` 或 `docs(comments)`

---

## 6. 测试规范(项目层)

补充 [`governance.md` §13](./governance.md#13-测试规范) 的项目级必测项。

### 6.1 测试位置(Rust)

| 类型 | 位置 | 何时用 |
| --- | --- | --- |
| 单元测试 | 与被测代码同文件末尾(`#[cfg(test)] mod tests`) | 测试某模块内部逻辑 |
| 集成测试 | 顶层 `tests/` 目录 | 跨模块 / CLI 入口行为 |
| 需要外部设施的测试 | 标记 `#[ignore]` + 注释说明所需设施 | 网络、第三方账号、付费 API |

### 6.2 必测项(本项目每模块必覆盖)

1. 配置加载 + 多账户解析(`<项目>:<模块>:<账户>` 命名空间)
2. **输出渲染:Text 与结构化(JSON)两种路径,含失败信封**——本项目 CLI 双输出契约
3. 错误类型到结构化信封的序列化
4. 每个核心执行单元:至少一个 happy-path 集成测试 + JSON 断言
5. **持久化层:upsert 幂等性、水位单调性、ID 重置、append-only 保留、token 边界匹配(GLOB)**

### 6.3 mock 纪律

- 网络调用(IMAP/SMTP/CalDAV/RSS/Notion)必须封装在 trait 背后,测试注入 fake
- 时间相关代码接受 `Clock` 函数对象或 `chrono::Clock` 抽象
- 随机 ID 接受 `gen_id` 函数对象或可重置计数器
- mock 与被测代码放在同一文件,命名清晰(如 `tests` 模块内)

### 6.4 性能预算

性能预算写入 F 类 ADR(冷启动 < 100ms / 网络超时 / 大输出流式);CI 不跑 perf 断言,改用结构约束(禁止全量加载、强制分页)。

---

## 7. 安全红线(项目层)

补充 [`governance.md` §14](./governance.md#14-安全红线) 的项目级红线。

### 7.1 凭证命名空间

- 命名空间规则固定为 `<项目>:<模块>:<账户>`(如 `everyday:mail:work`、`everyday:cal:personal`)
- 凭证由项目实际采用的安全凭据管理方式管理(keyring / 环境变量 / 加密文件等),实现细节写在项目安全 ADR
- 空凭证返回"未配置"错误,不 panic、不进入重试循环
- 缺失安全后端(headless 环境)必须返回明确错误,并允许交互式回退

### 7.2 本地文件操作

- 读取配置时 `PermissionDenied` 必须转为明确错误类型
- 写本地缓存数据库(mail_cache.db / timeline.db / memory.db 等)必须先确保父目录存在
- **不得直接使用未规范化的用户路径(禁止 `..` 注入)**——CLI 子命令从 `--config <path>` 等参数接收路径

### 7.3 输出与日志

- 不得打印完整的网络层内部字段(如 IMAP envelope 原始头、CalDAV etag、Notion block id 等)
- 结构化输出绝不内嵌凭证;认证失败消息只能说"账户 X 缺少凭证",不出现凭证本身
- 结构化输出失败时必须回退到通用信封(`{"error": "...", ...}`),不破坏对外契约

### 7.4 并发与时间陷阱(本项目踩坑)

- `tokio::spawn` 前必须先 `tokio::runtime::Handle::try_current()` 探测;runtime 关闭后 spawn 直接 panic 且 session 丢失
- DST 边界(春进秋退)的所有 `Local.from_local_datetime(&ndt).unwrap()` 必须改为 `.earliest()`(spring-forward gap 返回 None)或 `.latest()`(fall-back 返回 Some)
- 涉及本地时间 / 日历日 / 跨时区用户输入时,用显式歧义解析策略,不依赖 panic 类隐式崩溃路径

---

## 8. 编码风格(项目层)

补充 [`governance.md` §16](./governance.md#16-编码风格基线) 的项目级强制项。

### 8.1 强制项(Rust)

| 项 | 规则 |
| --- | --- |
| 格式化 | `cargo fmt`;CI 用 `--check` 验证 |
| 静态检查 | `cargo clippy --all-targets -- -D warnings`;CI 与本地同配置 |
| 公共 API | 必须有 `///` 文档注释(说明契约、不变量、副作用) |
| 模块 | 文件顶部必须有 `//!` 模块级文档注释 |
| 错误处理 | 非测试代码不使用 `unwrap()`/`expect()`;用 `Result<T, E>` + `?` 算子 + 上下文传递 |
| 构造器 | 凡能返回 `Result` 的构造器不 panic;遵循"成功构造或显式失败"原则 |

### 8.2 命名约定

- 模块/文件:领域概念命名(如 `mail`、`timeline`、`memory`),不用动词 / CLI 命令命名
- 类型:`PascalCase`
- 函数/变量:`snake_case`
- 常量:`SCREAMING_SNAKE_CASE`
- 错误类型以 `Error` 结尾(`AgentError`、`ConfigError` 等)

### 8.3 参数解析(CLI)

- 显式校验失败 → 明确错误类型,不静默回退默认值(见 L013)
- 全局模式 flag(`--json`)一次性检测,存入线程局部 `is_json()`,不重复扫描
- 单字符参数(`-x`)与带前缀参数(`--xxx`)的区别必须显式区分:`-x` 是值,`--xxx` 才是 flag

### 8.4 持久化与查询

- token 边界匹配使用 `GLOB`(锁定 token 边界),不用模糊子串 `LIKE`
- 配置路径数组索引访问需要支持自动扩展(避免误把缺失下标当成错误)
- 不同来源的同义配置键(布尔 `on/off`、`true/false`、`yes/no`)在配置加载层做规范归一,并在 ADR 中显式列出接受的别名

---

## 9. 发版与发版流水(项目层)

### 9.1 Release workflow 平台矩阵

本项目 release 在 GitHub Actions 触发,tag `v*` 触发以下矩阵:

| 平台 | 架构 | 产物 |
| --- | --- | --- |
| Linux | x86_64 | `everyday-vX.Y.Z-x86_64-unknown-linux-gnu.tar.xz` |
| macOS | x86_64 | `everyday-vX.Y.Z-x86_64-apple-darwin.tar.xz` |
| macOS | aarch64 | `everyday-vX.Y.Z-aarch64-apple-darwin.tar.xz` |
| Windows | x86_64 | `everyday-vX.Y.Z-x86_64-pc-windows-msvc.zip` |

> 共 4 平台 matrix。流程触发、注解 tag、推送规范见 `governance.md` §17。

### 9.2 发版流水表位置

发版流水表维护在 `progress.md` 的"发版流水"节,按版本号降序:

```markdown
## 发版流水
| 版本 | tag | 摘要 | 主相关 ADR |
| --- | --- | --- | --- |
| vX.Y.Z | `vX.Y.Z` | 一句话变更 | [ADR-id](docs/adr/...) |
```

### 9.3 推送到 GitHub 远端,其他镜像不推

- `origin` = `git@github.com:duyixian1234/everyday.git`(SSH)
- 内部镜像(cnb.cool 等)一律不推,避免无凭证推送导致失败

---

## 10. 新项目初始化清单(项目层)

补充 [`governance.md` §20](./governance.md#20-新项目初始化清单) 的 Everyday 项目层必备项。

### 10.1 必备文档(本项目实际)

- [ ] [`README.md`](./README.md) / [`README_ZH.md`](./README_ZH.md) — 用户文档
- [ ] [`agents.md`](./agents.md) — AI Agent 协作入口
- [ ] [`task_plan.md`](./task_plan.md) — 阶段 + Errors + 关键设计决策
- [ ] [`progress.md`](./progress.md) — 当前状态 + ADR 时间序索引 + 发版流水
- [ ] [`CONTEXT.md`](./CONTEXT.md) — 领域术语表
- [ ] [`governance.md`](./governance.md) — 通用治理方法论
- [ ] [`everyday-conventions.md`](./everyday-conventions.md) — 项目级约定(本文件)
- [ ] [`docs/adr/README.md`](./docs/adr/README.md) — ADR 索引
- [ ] [`.rules/RULES.md`](./.rules/RULES.md) — 规则目录索引

### 10.2 必备工具配置(本项目实际)

- [ ] [`Justfile`](./Justfile) — `format` / `check` / `test` / `build` / `ci` / `check-links`
- [ ] CI:GitHub Actions `.github/workflows/release.yml`(4 平台 matrix)
- [ ] 跨文档链接完整性脚本:`scripts/check-doc-links.sh`
- [ ] `Cargo.toml` + `Cargo.lock`(锁文件入库)

### 10.3 必备 ADR(本项目首次发版前)

至少 3 篇 ADR:

1. **CLI 形态** — 描述 `everyday <module> <action>` 命令结构、Text/JSON 双输出
2. **凭证存储** — 描述多账户命名空间 `<项目>:<模块>:<账户>` 与所选安全后端
3. **错误模型** — 描述 `AgentError` 枚举到结构化信封的映射

### 10.4 `.rules/` 规则目录(本项目实际)

| 文件 | 主题 |
| --- | --- |
| `RULES.md` | 索引 |
| `01-workflow.md` | 开发工作流 |
| `02-coding-style.md` | 编码风格 |
| `03-testing.md` | 测试规范 |
| `04-security.md` | 安全红线 |
| `05-commit.md` | 提交规范 |
| `06-justfile.md` | Justfile 约定 |
| `07-dependency-pitfalls.md` | 依赖踩坑日志 |

---

_本文件与 `governance.md` 并行维护。任何项目级新约束,先评估"是否可通用"——
可通用 → 入 `governance.md`;仅适用本项目 → 入本文档。冲突时以本文档为准。_