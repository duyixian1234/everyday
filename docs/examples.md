# Usage Examples

Copy-paste recipes per module. This is the complete version of the README
"Usage Examples" section.

- [English](examples.md) · [中文](examples_zh.md)

---

### Mail

```bash
# List all folders
everyday mail folders

# View the 10 most recent unread messages (JSON)
everyday mail list --unread --limit 10 --json

# Search messages in a specific folder
everyday mail search --query "invoice" --folder INBOX --json

# Read a message
everyday mail read 12345 --json

# Send a message
everyday mail send \
  --to recipient@example.com \
  --subject "Weekly report" \
  --body "Summary of this week's work..." \
  --cc manager@example.com

# Switch account
everyday mail list --account personal --json
```

### Config

```bash
# Initialize
everyday config init

# Show config
everyday config list

# Read an item
everyday config get mail.accounts.0.username

# Modify an item
everyday config set mail.accounts.0.smtp_port 465

# Verify
everyday config get mail.accounts.0.smtp_port
```

### Notes (local SQLite by default)

```bash
# Search pages / databases (JSON)
everyday note search --query "work" --json

# List pages
everyday note list --json

# Create a record in a database with multiple properties
everyday note create \
  --title "A Deep Dive into Rust Async Runtimes" \
  --prop "Type:Article" \
  --prop "Status:Unread" \
  --prop "URL:https://..."

# Read a page body (aggregated into Markdown)
everyday note read <id> --json

# Append a quick note to the default scratch page (id optional)
everyday note append --text "### Auto-captured by AI
Found a competitor link in message 12345: https://..."

# Append via pipe
echo "Batch-captured content" | everyday note append <id>

# Update page properties
everyday note update <id> --prop "Status:Read"
```

### To-dos (local SQLite by default)

```bash
# The local provider needs no login — just add / list (tables auto-created)

# List unfinished tasks (by Due ascending)
everyday todo list --json

# All tasks (including completed)
everyday todo list --all --json

# Add a task
everyday todo add --title "Write weekly report" --due 2026-07-15 --priority P1

# Status transitions (returns the task id)
everyday todo start <id>
everyday todo complete <id>
```

### Bookmarks (local SQLite by default)

```bash
# The local provider needs no login — just add / list (tables auto-created)

# Add a bookmark with tags
everyday bookmark add \
  --url "https://www.rust-lang.org" \
  --title "The Rust Programming Language" \
  --tags "rust,lang"

# List all bookmarks (JSON)
everyday bookmark list --json

# Filter by a single tag
everyday bookmark list --tag rust
```

