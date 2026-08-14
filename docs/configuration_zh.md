# 配置

`config.toml` 结构、凭证安全规则与多账户配置。本文件是根 README「配置」章节的完整版。

- [English](configuration.md) · [中文](configuration_zh.md)

---

配置文件路径：`~/.config/everyday/config.toml`

```toml
[default_account]
mail = "work"
calendar = "personal"
note = "personal"
bookmark = "personal"

[[mail.accounts]]
name = "work"
imap_host = "imap.example.com"
imap_port = 993          # 默认 993
smtp_host = "smtp.example.com"
smtp_port = 587          # 默认 587
username = "me@example.com"
tls = true               # 默认 true

[[mail.accounts]]
name = "personal"
imap_host = "imap.gmail.com"
imap_port = 993
smtp_host = "smtp.gmail.com"
smtp_port = 587
username = "me@gmail.com"
tls = true

[[calendar.accounts]]
name = "personal"
caldav_url = "https://caldav.example.com/me"
username = "me"

[[rss.feeds]]
name = "hackernews"
url = "https://hnrss.org/frontpage"
category = "tech"

# 笔记 / 待办默认本地 SQLite provider，开箱即用、无需凭证
[[note.accounts]]
name = "personal"
provider = "local"
# db_path = "/absolute/path/to/notes.db"   # 可选，缺省 ~/.config/everyday/note-personal.db

[[todo.accounts]]
name = "personal"
provider = "local"
# db_path = "/absolute/path/to/todos.db"   # 可选，缺省 ~/.config/everyday/todo-personal.db

[[bookmark.accounts]]
name = "personal"
provider = "local"
# db_path = "/absolute/path/to/bookmarks.db"   # 可选，缺省 ~/.config/everyday/bookmark-personal.db
```

### 凭证安全

密码**绝不**存储在配置文件中，而是通过系统密钥环管理：

- **keyring 服务名约定**：`everyday/<module>/<account>`（如 `everyday/mail/work`）
- **存储凭证**：`everyday auth login --module mail --account work`（交互式输入，存入密钥环）
- **读取凭证**：模块通过 `auth::get_credential` 自动从密钥环读取，无需手动指定
- **环境变量回退（可选，R020）**：headless 环境（无 keyring 后端）开启 `[auth] env_credentials = true` 或 `EVERYDAY_ENV_CREDENTIALS=1` 后，可 export `EVERYDAY_<MODULE>_<ACCOUNT>_PASSWORD` 提供凭据；读取链 keyring → env → 报错

### 多账户

每个模块支持多个命名账户：

- 配置文件中通过 `[[mail.accounts]]` 等数组定义
- `[default_account]` 指定各模块的默认账户名
- `--account NAME` 覆盖默认账户

