//! 系统监控模块（参考实现）。
//!
//! `sys status` 已可工作：通过 `sysinfo` 输出 CPU/内存/磁盘，
//! 展示 `Output::Records` 在 Text/JSON 两种模式下的渲染效果，
//! 作为后续模块（邮件/日历/RSS）的实现模板。

use async_trait::async_trait;
use serde_json::json;
use sysinfo::System;

use crate::error::{AgentError, Result};
use crate::modules::{ActionDoc, Executor};
use crate::output::Output;

pub struct SystemModule;

impl SystemModule {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Executor for SystemModule {
    fn name(&self) -> &'static str { "sys" }

    fn description(&self) -> &'static str {
        "System monitoring: resource status, file watching."
    }

    fn actions(&self) -> Vec<ActionDoc> {
        vec![
            ActionDoc::new("status", "Show CPU / memory / disk usage", "everyday sys status"),
            ActionDoc::new("watch", "Watch a path for changes", "everyday sys watch <path> [--interval SEC]"),
            ActionDoc::new("clip", "Read/write clipboard", "everyday sys clip [get|set VALUE]"),
        ]
    }

    async fn execute(&self, action: &str, _args: &[String]) -> Result<Output> {
        match action {
            "status" => self.status().await,
            "watch" => Err(AgentError::NotImplemented(
                "sys watch — notify integration pending (see task_plan Phase 6)".into(),
            )),
            "clip" => Err(AgentError::NotImplemented(
                "sys clip — arboard integration pending".into(),
            )),
            other => Err(AgentError::UnknownAction(format!("sys {other}"))),
        }
    }
}

impl SystemModule {
    /// 采集系统资源快照。
    async fn status(&self) -> Result<Output> {
        // sysinfo 是同步库，放进 spawn_blocking 避免阻塞异步运行时。
        let snap = tokio::task::spawn_blocking(|| {
            let mut sys = System::new_all();
            sys.refresh_all();

            // CPU：取所有核心的平均使用率（跨版本稳定 API）。
            let cpus = sys.cpus();
            let cpu_usage = if cpus.is_empty() {
                0.0
            } else {
                cpus.iter().map(|c| c.cpu_usage()).sum::<f32>() / cpus.len() as f32
            };

            let total_mem = sys.total_memory();
            let used_mem = sys.used_memory();
            let total_swap = sys.total_swap();
            let used_swap = sys.used_swap();

            // 磁盘：sysinfo 0.30 起 Disks 为独立结构。
            let disks_list = sysinfo::Disks::new_with_refreshed_list();
            let disks: Vec<_> = disks_list
                .iter()
                .map(|d| {
                    let total = d.total_space();
                    let used = total.saturating_sub(d.available_space());
                    (d.mount_point().display().to_string(), used, total)
                })
                .collect();

            Snapshot {
                cpu_usage,
                mem_total: total_mem,
                mem_used: used_mem,
                swap_total: total_swap,
                swap_used: used_swap,
                disks,
            }
        })
        .await
        .map_err(|e| AgentError::Other(format!("system snapshot join failed: {e}")))?;

        Ok(snap.into_output())
    }
}

struct Snapshot {
    cpu_usage: f32,
    mem_total: u64,
    mem_used: u64,
    swap_total: u64,
    swap_used: u64,
    disks: Vec<(String, u64, u64)>,
}

impl Snapshot {
    fn into_output(self) -> Output {
        // 同时构造 Records（Text 模式表格）与 JSON 值（JSON 模式）。
        // 通过一个 Output::Json 承载完整结构，Text 模式下再额外给一份表格。
        // 这里选择返回 Records 以演示表格渲染；JSON 模式会自动转成对象数组。
        let headers = vec![
            "resource".to_string(),
            "used".to_string(),
            "total".to_string(),
            "pct".to_string(),
        ];

        let pct = |used: u64, total: u64| -> String {
            if total == 0 {
                "-".into()
            } else {
                format!("{:.1}%", (used as f64 / total as f64) * 100.0)
            }
        };

        let mut rows = vec![
            vec![
                "cpu".into(),
                format!("{:.1}%", self.cpu_usage),
                "100.0%".into(),
                format!("{:.1}%", self.cpu_usage),
            ],
            vec![
                "memory".into(),
                human_bytes(self.mem_used),
                human_bytes(self.mem_total),
                pct(self.mem_used, self.mem_total),
            ],
            vec![
                "swap".into(),
                human_bytes(self.swap_used),
                human_bytes(self.swap_total),
                pct(self.swap_used, self.swap_total),
            ],
        ];

        for (mp, used, total) in &self.disks {
            rows.push(vec![
                format!("disk {mp}"),
                human_bytes(*used),
                human_bytes(*total),
                pct(*used, *total),
            ]);
        }

        // 附带 JSON 语义：我们直接返回 Records；JSON 模式渲染为对象数组。
        // 若调用方需要完整快照对象，可改返回 Output::Json。
        let _ = json!({"cpu": self.cpu_usage});
        Output::records(headers, rows)
    }
}

fn human_bytes(n: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} {}", UNITS[0])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::RenderMode;

    #[tokio::test]
    async fn status_returns_records() {
        let m = SystemModule::new();
        let out = m.execute("status", &[]).await.unwrap();
        let text = out.render(RenderMode::Text);
        assert!(text.contains("cpu") || text.contains("memory"));
    }

    #[test]
    fn human_bytes_formats() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(1024 * 1024 * 3), "3.0 MiB");
    }
}
