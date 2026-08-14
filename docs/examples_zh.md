# 使用示例

各模块可复制的命令示例。本文件是根 README「使用示例」章节的完整版。

- [English](examples.md) · [中文](examples_zh.md)

---

### 邮件

```bash
# 列出所有文件夹
everyday mail folders

# 查看最近 10 封未读邮件（JSON）
everyday mail list --unread --limit 10 --json

# 在指定文件夹中查找邮件
everyday mail search --query "invoice" --folder INBOX --json

# 读取某封邮件
everyday mail read 12345 --json

# 发送邮件
everyday mail send \
  --to recipient@example.com \
  --subject "周报" \
  --body "本周工作总结..." \
  --cc manager@example.com

# 切换账户
everyday mail list --account personal --json
```

### 配置

```bash
# 初始化
everyday config init

# 查看配置
everyday config list

# 读取某项
everyday config get mail.accounts.0.username

# 修改某项
everyday config set mail.accounts.0.smtp_port 465

# 验证
everyday config get mail.accounts.0.smtp_port
```

### 笔记（默认本地 SQLite）

```bash
# 搜索页面 / 数据库（JSON）
everyday note search --query "工作" --json

# 列出页面
everyday note list --json

# 在数据库中新建一条记录，含多项属性
everyday note create \
  --title "Rust 异步运行时深入浅出" \
  --prop "类型:文章" \
  --prop "状态:未读" \
  --prop "URL:https://..."

# 读取页面正文（聚合成 Markdown）
everyday note read <id> --json

# 向默认速记页面追加一条闪念（id 可选）
everyday note append --text "### AI 自动捕获
在 12345 号邮件发现竞品链接：https://..."

# 管道方式追加
echo "批量捕获的内容" | everyday note append <id>

# 修改页面属性
everyday note update <id> --prop "状态:已读"
```

### 待办（默认本地 SQLite）

```bash
# 本地 provider 无需登录，直接 add / list 即可（表自动创建）

# 列出未完成任务（按 Due 升序）
everyday todo list --json

# 全部任务（含已完成）
everyday todo list --all --json

# 新增任务
everyday todo add --title "写周报" --due 2026-07-15 --priority P1

# 状态切换（返回任务 id）
everyday todo start <id>
everyday todo complete <id>
```

### 书签（默认本地 SQLite）

```bash
# 本地 provider 无需登录，直接 add / list 即可（表自动创建）

# 新增带标签的书签
everyday bookmark add \
  --url "https://www.rust-lang.org" \
  --title "The Rust Programming Language" \
  --tags "rust,lang"

# 列出全部书签（JSON）
everyday bookmark list --json

# 按单个标签过滤
everyday bookmark list --tag rust
```

