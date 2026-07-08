### Everyday 项目 PRD (V2.0)

**项目名称：** Everyday
**项目口号：** The Rust-powered hands for your AI Agent.
**项目类型：** CLI (命令行界面) 工具
**目标平台：** Windows, macOS, Linux

---

#### 1. 产品愿景
打造一款高性能、内存安全的本地 CLI 工具集，作为 AI Agent 的“数字双手”。它不仅覆盖基础的通讯与日程管理，更能深入操作系统层面，提供系统感知、网络交互和文件处理能力，使 AI 能够真正地在本地环境执行复杂的自动化工作流。

#### 2. 核心用户
*   **主要用户：** AI Agents (如 AutoGPT, LangChain 应用, 自定义脚本)
*   **次要用户：** 开发者、系统管理员、自动化爱好者

#### 3. 功能模块详述 (Scope)

| 模块分类 | 子模块 | 协议/库依赖 | 核心命令示例 | 描述 |
| :--- | :--- | :--- | :--- | :--- |
| **通讯协作** | **邮件管理** | IMAP, SMTP (`async-imap`, `lettre`) | `mail list --unread`, `mail send` | 邮件的查收、发送、搜索与附件处理。 |
| | **日历管理** | CalDAV (`caldav`) | `cal list --today`, `cal add` | 日程的增删改查，支持提醒。 |
| | **资讯订阅** | RSS/Atom (`feed-rs`) | `rss follow`, `rss digest` | 订阅源管理，聚合内容摘要。 |
| **系统层** | **系统监控** | System Info (`sysinfo`), Notify (`notify`) | `sys status`, `fs watch ./logs` | 查看硬件资源占用，监听文件系统变化。 |
| | **剪贴板** | Clipboard (`arboard`, `copypasta`) | `clip get`, `clip set "text"` | 读写系统剪贴板，用于跨应用数据流转。 |
| **网络层** | **网页抓取** | HTTP Client (`reqwest`), HTML Parser (`scraper`) | `net fetch <url>` | 获取网页内容，自动清洗为纯净 Markdown/Text。 |
| | **HTTP 工具** | Reqwest | `net request --method POST --body '...'` | 通用的 REST API 调用客户端。 |
| **文件层** | **文件操作** | Walkdir, Ignore (`ignore`), Regex | `fs search --content "error"`, `fs tree` | 强大的文件查找（支持内容搜索）、目录树展示。 |
| | **文件内容** | Serde (`serde_json`, `toml`) | `fs read-json config.toml` | 结构化读取和解析常见配置文件。 |

#### 4. 命令行设计规范 (UX)
采用统一的 **`everyday <module> <action> [options]`** 结构。

*   **统一输出接口：**
    *   **默认模式：** 人类可读的表格或文本（适用于终端直接查看）。
    *   **JSON 模式：** 添加 `--json` 参数，输出纯净 JSON。**这是 AI Agent 交互的主要模式**。
*   **配置管理：**
    *   配置文件：`~/.config/everyday/config.toml`
    *   命令：`everyday config set mail.user "xxx"`
*   **错误处理：**
    *   退出码：成功为 0，失败为非 0。
    *   JSON 错误格式：`{"error": "ErrorType", "message": "Details..."}`

#### 5. 技术架构与依赖

```text
src/
├── main.rs          // 入口点
├── cli.rs           // Clap 参数定义
├── config.rs        // 配置加载与验证
├── error.rs         // 统一错误定义
├── output.rs        // 输出格式化 (Text/JSON)
└── modules/
    ├── mod.rs       // 模块注册表
    ├── email.rs     // 邮件模块
    ├── calendar.rs  // 日历模块
    ├── rss.rs       // RSS模块
    ├── system.rs    // 系统监控模块
    ├── network.rs   // 网络工具模块
    └── fs.rs        // 文件系统模块
```

**核心依赖推荐：**
*   **异步运行时：** `tokio`
*   **CLI 解析：** `clap` (with `derive` feature)
*   **序列化：** `serde`, `serde_json`
*   **系统信息：** `sysinfo`
*   **文件搜索：** `ignore` (ripgrep 作者出品，支持 .gitignore)
*   **网页抓取：** `reqwest`, `scraper`
*   **配置：** `toml`, `dirs-next`

#### 6. 非功能性需求
*   **安全性：**
    *   **凭证管理：** 严禁明文存储密码。使用 `keyring` crate 对接系统密钥环（macOS Keychain, Windows Credential Manager, Linux Secret Service）。
    *   **权限最小化：** 仅在需要时申请权限。
*   **性能：**
    *   冷启动时间 < 100ms。
    *   文件搜索和网络请求需支持异步流式处理，避免大文件阻塞。
*   **健壮性：**
    *   网络请求必须设置超时。
    *   文件系统操作需处理权限不足的情况。

#### 7. 未来扩展 (Roadmap)
*   **MCP 兼容层：** 实现 Model Context Protocol，使其能被 Claude Desktop 等客户端原生调用。
*   **插件系统：** 允许用户编写 WASM 插件扩展功能。
*   **数据库交互：** 增加 `db query` 模块，支持 SQLite/PostgreSQL 的简单查询。
