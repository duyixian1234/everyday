# Configuration

`config.toml` layout, credential safety rules, and multi-account setup. This is
the complete version of the README "Configuration" section.

- [English](configuration.md) · [中文](configuration_zh.md)

---

Config file path: `~/.config/everyday/config.toml`

```toml
[default_account]
mail = "work"
calendar = "personal"
note = "personal"
bookmark = "personal"

[[mail.accounts]]
name = "work"
imap_host = "imap.example.com"
imap_port = 993          # default 993
smtp_host = "smtp.example.com"
smtp_port = 587          # default 587
username = "me@example.com"
tls = true               # default true

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

# Notes / to-dos default to the local SQLite provider — works out of the box, no credentials
[[note.accounts]]
name = "personal"
provider = "local"
# db_path = "/absolute/path/to/notes.db"   # optional, defaults to ~/.config/everyday/note-personal.db

[[todo.accounts]]
name = "personal"
provider = "local"
# db_path = "/absolute/path/to/todos.db"   # optional, defaults to ~/.config/everyday/todo-personal.db

[[bookmark.accounts]]
name = "personal"
provider = "local"
# db_path = "/absolute/path/to/bookmarks.db"   # optional, defaults to ~/.config/everyday/bookmark-personal.db
```

### Credential safety

Passwords are **never** stored in the config file; they are managed through the system keyring:

- **keyring service naming**: `everyday/<module>/<account>` (e.g. `everyday/mail/work`)
- **Store a credential**: `everyday auth login --module mail --account work` (interactive input; password stored in the keyring)
- **Read a credential**: the module reads it from the keyring automatically via `auth::get_credential` — no manual step needed
- **Env fallback (opt-in, R020)**: on headless systems (no keyring backend), enable `[auth] env_credentials = true` or `EVERYDAY_ENV_CREDENTIALS=1` and export `EVERYDAY_<MODULE>_<ACCOUNT>_PASSWORD`; read chain keyring → env → error

### Multiple accounts

Each module supports multiple named accounts:

- Defined via arrays such as `[[mail.accounts]]` in the config file
- `[default_account]` specifies the default account name per module
- `--account NAME` overrides the default

