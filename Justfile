# Everyday — 开发流程管理（just）
# 运行 `just` 查看可用命令列表。
# 跨平台终端：Unix 用 bash，Windows 用 powershell.exe。

set shell := ["bash", "-c"]
set windows-shell := ["powershell.exe", "-NoProfile", "-NoLogo", "-Command"]

# 列出所有可用命令
default:
    @just --list

# 格式化所有代码（cargo fmt）
format:
    cargo fmt

# 检查格式与 lint（fmt --check && clippy）
check:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings

# 运行测试（cargo test）
test:
    cargo test

# 构建项目（cargo build）
build:
    cargo build

# 完整 CI 流程：check && test && build
ci: check test build
