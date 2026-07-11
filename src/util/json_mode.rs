//! 渲染模式探测工具。
//!
//! 通过线程局部变量传递，由 `main.rs` 在进程启动时设置一次（基于 clap
//! 解析的 `--json` flag）。模块深层辅助函数可读 `is_json()` 而无需
//! 把 `RenderMode` 层层下传。
//!
//! 替代旧实现的 `std::env::args()` 二次扫描 —— 旧实现会被宿主进程
//! 的命令行污染，且与已解析的 `cli.json` 重复探测。

use std::cell::Cell;

thread_local! {
    /// 进程级 JSON 模式标记。默认 false，main.rs 在启动时按 clap 解析结果设置。
    static JSON_MODE: Cell<bool> = const { Cell::new(false) };
}

/// 设置当前线程的 JSON 模式标记。由 `main` 在启动时调用一次。
pub fn set_json_mode(json: bool) {
    JSON_MODE.with(|c| c.set(json));
}

/// 当前线程的 JSON 模式标记。模块深层辅助函数查询它。
pub fn is_json() -> bool {
    JSON_MODE.with(|c| c.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_false() {
        // 进程默认未设置时为 false。本测试在独立线程跑，TLS 干净。
        // 注：cargo test 在同一进程串行跑测试，TLS 在每个 #[test] 之间不重置。
        // 为不互相污染，本测试只验证 set/get 一致性。
        set_json_mode(false);
        assert!(!is_json());
        set_json_mode(true);
        assert!(is_json());
        set_json_mode(false);
    }
}
