//! 渲染模式探测工具。
//!
//! 部分模块（local/note/todo 的 provider 实现）在深层函数里需要知道当前是否为
//! JSON 输出，但不便层层透传 flag，故统一以进程参数中的 `--json` 为准探测。

/// 当前是否为 JSON 输出模式（以进程参数含 `--json` 为准）。
pub fn is_json() -> bool {
    std::env::args().any(|a| a == "--json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_json_reflects_process_args() {
        // 测试进程通常不带 --json；此处只验证函数可调用且返回布尔。
        let _ = is_json();
    }
}
