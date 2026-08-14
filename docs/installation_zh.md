# 安装

各平台完整安装步骤：一键安装脚本、预编译二进制、源码构建、校验和与验证。
本文件是根 README「安装」章节的完整版。

- [English](installation.md) · [中文](installation_zh.md)

---

### 安装脚本（推荐）

Release 流水线由 [cargo-dist](https://axodotdev.github.io/cargo-dist) 托管——每个 Release 附带安装脚本、压缩包、校验和与 [Sigstore 签名证明](https://github.com/sigstore/sigstore)（验证：`gh attestation verify <file> --repo duyixian1234/everyday`）。

一行安装（自动取 latest）：

```bash
# macOS / Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/duyixian1234/everyday/releases/latest/download/everyday-installer.sh | sh
```

```powershell
# Windows（PowerShell）
powershell -ExecutionPolicy Bypass -c "irm https://github.com/duyixian1234/everyday/releases/latest/download/everyday-installer.ps1 | iex"
```

### 下载预编译二进制

从 [GitHub Releases](https://github.com/duyixian1234/everyday/releases) 下载对应平台的压缩包，解压后将 `everyday` 加入 `PATH` 即可。每个 Release 附带各平台（含 macOS x86_64 与 Apple Silicon / aarch64）的资产：

| 平台 | 资产文件 | 解压 / 安装 |
|------|----------|-------------|
| Linux (x86_64) | `everyday-x86_64-unknown-linux-gnu.tar.xz` | `tar xJf <file> && sudo mv everyday /usr/local/bin/` |
| macOS (x86_64) | `everyday-x86_64-apple-darwin.tar.xz` | `tar xJf <file> && sudo mv everyday /usr/local/bin/` |
| macOS (Apple Silicon / aarch64) | `everyday-aarch64-apple-darwin.tar.xz` | `tar xJf <file> && sudo mv everyday /usr/local/bin/` |
| Windows (x86_64) | `everyday-x86_64-pc-windows-msvc.zip` | 解压后将 `everyday.exe` 放入 `PATH` 目录 |

> 二进制由 CI 在每次打 `v*` tag 时自动构建并发布（见 `.github/workflows/release.yml`，由 cargo-dist 生成），覆盖 Linux / macOS（x86_64 与 aarch64）/ Windows 三平台四架构。每个 Release 另附 `sha256.sum` 与各资产校验和。

### 从源码构建

```bash
git clone https://github.com/duyixian1234/everyday.git
cd everyday
cargo build --release
```

编译产物位于 `target/release/everyday`，将其加入 `PATH` 即可。

### 通过 cargo 安装

```bash
cargo install --git https://github.com/duyixian1234/everyday.git
```

### 验证安装

```bash
everyday --version
everyday config path
```

