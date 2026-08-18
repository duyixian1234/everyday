//! No-shell subprocess runner with timeout, process-tree termination, teeing,
//! and bounded capture (ADR F017).

use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};

use crate::config::TaskConfig;
use crate::error::{AgentError, Result};
use crate::modules::task::store::{TaskRunRecord, TaskStore};

const CAPTURE_LIMIT: usize = 64 * 1024;
const TRUNCATION_MARKER: &[u8] = b"\n...[truncated at 65536 bytes]";

/// Where live child output is relayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayMode {
    /// stdout→stdout and stderr→stderr.
    Terminal,
    /// Both streams→stderr, preserving stdout for a JSON result.
    Structured,
    /// Capture only; used by scheduled runs.
    Silent,
}

/// Execute one configured task and persist its result.
pub async fn run(
    store: &TaskStore,
    task_name: &str,
    task: &TaskConfig,
    extra_args: &[String],
    force_capture: bool,
    relay: RelayMode,
) -> Result<TaskRunRecord> {
    if !extra_args.is_empty() && !task.allow_extra_args {
        return Err(AgentError::InvalidArgument(format!(
            "task `{task_name}` does not allow extra arguments"
        )));
    }

    let configured_args: Vec<String> = task.args.split_whitespace().map(str::to_string).collect();
    let mut resolved_args = configured_args.clone();
    resolved_args.extend(extra_args.iter().cloned());
    let capture_output = force_capture || task.capture_output;
    let started_at = chrono::Utc::now();
    let started = Instant::now();
    let cwd = std::env::current_dir()?.display().to_string();

    let mut command = Command::new(&task.command);
    command.args(&resolved_args).kill_on_drop(true);
    configure_process_group(&mut command);

    let pipe_output = capture_output || relay != RelayMode::Terminal;
    if pipe_output {
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
    } else {
        command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    }
    if relay == RelayMode::Silent {
        command.stdin(Stdio::null());
    } else {
        command.stdin(Stdio::inherit());
    }

    let spawn = command.spawn();
    let record = match spawn {
        Ok(mut child) => {
            let stdout_reader = child
                .stdout
                .take()
                .map(|stream| tokio::spawn(read_stream(stream, StreamKind::Stdout, relay)));
            let stderr_reader = child
                .stderr
                .take()
                .map(|stream| tokio::spawn(read_stream(stream, StreamKind::Stderr, relay)));

            let (status, timed_out) = wait_with_timeout(&mut child, task.timeout_secs).await?;
            let stdout = join_capture(stdout_reader).await?;
            let stderr = join_capture(stderr_reader).await?;
            let exit_code = status.and_then(|s| s.code());
            let status_name = if timed_out {
                "timeout"
            } else if status.is_some_and(|s| s.success()) {
                "success"
            } else {
                "failed"
            };
            TaskRunRecord {
                id: crate::util::id::gen_id("tk"),
                task_name: task_name.to_string(),
                command: task.command.clone(),
                args: configured_args,
                extra_args: (!extra_args.is_empty()).then(|| extra_args.to_vec()),
                resolved_args,
                allow_extra_args: task.allow_extra_args,
                timeout_secs: task.timeout_secs,
                capture_output,
                cwd,
                status: status_name.to_string(),
                exit_code,
                timed_out,
                stdout: capture_output.then_some(stdout),
                stderr: capture_output.then_some(stderr),
                started_at: started_at.to_rfc3339(),
                duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            }
        }
        Err(error) => {
            let message = format!("failed to start `{}`: {error}", task.command);
            if relay != RelayMode::Silent {
                let mut stderr = tokio::io::stderr();
                let _ = stderr.write_all(format!("{message}\n").as_bytes()).await;
                let _ = stderr.flush().await;
            }
            TaskRunRecord {
                id: crate::util::id::gen_id("tk"),
                task_name: task_name.to_string(),
                command: task.command.clone(),
                args: configured_args,
                extra_args: (!extra_args.is_empty()).then(|| extra_args.to_vec()),
                resolved_args,
                allow_extra_args: task.allow_extra_args,
                timeout_secs: task.timeout_secs,
                capture_output,
                cwd,
                status: "failed".into(),
                exit_code: Some(1),
                timed_out: false,
                stdout: capture_output.then(String::new),
                stderr: capture_output.then_some(message),
                started_at: started_at.to_rfc3339(),
                duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            }
        }
    };
    store.insert_run(&record).await?;
    Ok(record)
}

async fn wait_with_timeout(
    child: &mut Child,
    timeout_secs: u64,
) -> Result<(Option<std::process::ExitStatus>, bool)> {
    if timeout_secs == 0 {
        return Ok((Some(child.wait().await?), false));
    }
    match tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait()).await {
        Ok(status) => Ok((Some(status?), false)),
        Err(_) => {
            kill_process_tree(child).await;
            let _ = child.wait().await;
            Ok((None, true))
        }
    }
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.as_std_mut().process_group(0);
}

#[cfg(windows)]
fn configure_process_group(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command
        .as_std_mut()
        .creation_flags(CREATE_NEW_PROCESS_GROUP);
}

#[cfg(unix)]
async fn kill_process_tree(child: &mut Child) {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    const SIGKILL: i32 = 9;
    if let Some(pid) = child.id() {
        // The child is the leader of a dedicated process group.
        unsafe {
            kill(-(pid as i32), SIGKILL);
        }
    }
    let _ = child.start_kill();
}

#[cfg(windows)]
async fn kill_process_tree(child: &mut Child) {
    if let Some(pid) = child.id() {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
    let _ = child.start_kill();
}

#[derive(Debug, Clone, Copy)]
enum StreamKind {
    Stdout,
    Stderr,
}

async fn read_stream<R>(mut reader: R, kind: StreamKind, relay: RelayMode) -> Result<String>
where
    R: AsyncRead + Unpin,
{
    let mut captured = Vec::new();
    let mut truncated = false;
    let mut chunk = [0u8; 8192];
    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        relay_chunk(&chunk[..n], kind, relay).await?;
        if captured.len() < CAPTURE_LIMIT {
            let remaining = CAPTURE_LIMIT - captured.len();
            let take = remaining.min(n);
            captured.extend_from_slice(&chunk[..take]);
            truncated |= take < n;
        } else {
            truncated = true;
        }
    }
    if truncated {
        captured.extend_from_slice(TRUNCATION_MARKER);
    }
    Ok(String::from_utf8_lossy(&captured).into_owned())
}

async fn relay_chunk(chunk: &[u8], kind: StreamKind, relay: RelayMode) -> Result<()> {
    match relay {
        RelayMode::Silent => Ok(()),
        RelayMode::Structured => {
            let mut writer = tokio::io::stderr();
            writer.write_all(chunk).await?;
            writer.flush().await?;
            Ok(())
        }
        RelayMode::Terminal => match kind {
            StreamKind::Stdout => {
                let mut writer = tokio::io::stdout();
                writer.write_all(chunk).await?;
                writer.flush().await?;
                Ok(())
            }
            StreamKind::Stderr => {
                let mut writer = tokio::io::stderr();
                writer.write_all(chunk).await?;
                writer.flush().await?;
                Ok(())
            }
        },
    }
}

async fn join_capture(task: Option<tokio::task::JoinHandle<Result<String>>>) -> Result<String> {
    match task {
        Some(task) => task
            .await
            .map_err(|e| AgentError::Other(format!("task output reader failed: {e}")))?,
        None => Ok(String::new()),
    }
}

/// Exit code exposed by the CLI.
pub fn mirrored_exit_code(record: &TaskRunRecord) -> i32 {
    if record.timed_out {
        124
    } else {
        record.exit_code.unwrap_or(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::LazyLock;

    static CHILD_LOCK: LazyLock<tokio::sync::Mutex<()>> =
        LazyLock::new(|| tokio::sync::Mutex::new(()));

    fn db_path(name: &str) -> PathBuf {
        std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("task-runner-{}-{name}.db", std::process::id()))
    }

    fn helper_task(timeout_secs: u64, capture_output: bool) -> TaskConfig {
        TaskConfig {
            command: std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            args: "--ignored --exact modules::task::runner::tests::task_child_helper --nocapture"
                .into(),
            allow_extra_args: true,
            timeout_secs,
            capture_output,
            schedule: None,
        }
    }

    fn set_child_mode(mode: &str) {
        unsafe {
            std::env::set_var("EVERYDAY_TASK_CHILD_MODE", mode);
        }
    }

    fn clear_child_mode() {
        unsafe {
            std::env::remove_var("EVERYDAY_TASK_CHILD_MODE");
            std::env::remove_var("EVERYDAY_TASK_CHILD_FILE");
        }
    }

    #[test]
    #[ignore = "spawned explicitly by runner tests"]
    fn task_child_helper() {
        match std::env::var("EVERYDAY_TASK_CHILD_MODE").as_deref() {
            Ok("exit7") => std::process::exit(7),
            Ok("large") => print!("{}", "x".repeat(70 * 1024)),
            Ok("sleep") => std::thread::sleep(Duration::from_secs(5)),
            Ok("tree-parent") => {
                let file = std::env::var("EVERYDAY_TASK_CHILD_FILE").unwrap();
                let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--ignored",
                        "--exact",
                        "modules::task::runner::tests::task_child_helper",
                        "--nocapture",
                    ])
                    .env("EVERYDAY_TASK_CHILD_MODE", "tree-grandchild")
                    .env("EVERYDAY_TASK_CHILD_FILE", file)
                    .spawn()
                    .unwrap();
                let _ = child.wait();
            }
            Ok("tree-grandchild") => {
                std::thread::sleep(Duration::from_secs(4));
                std::fs::write(std::env::var("EVERYDAY_TASK_CHILD_FILE").unwrap(), "alive")
                    .unwrap();
            }
            _ => {}
        }
    }

    #[tokio::test]
    async fn mirrors_nonzero_exit_and_records_resolved_args() {
        let _guard = CHILD_LOCK.lock().await;
        set_child_mode("exit7");
        let path = db_path("exit");
        let _ = std::fs::remove_file(&path);
        let store = TaskStore::open_path(&path).await.unwrap();
        let extra = vec!["extra".to_string()];
        let record = run(
            &store,
            "exit",
            &helper_task(10, true),
            &extra,
            false,
            RelayMode::Silent,
        )
        .await
        .unwrap();
        assert_eq!(record.status, "failed");
        assert_eq!(mirrored_exit_code(&record), 7);
        assert_eq!(
            record.resolved_args.last().map(String::as_str),
            Some("extra")
        );
        clear_child_mode();
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn rejects_extra_args_when_disabled() {
        let path = db_path("reject");
        let _ = std::fs::remove_file(&path);
        let store = TaskStore::open_path(&path).await.unwrap();
        let mut task = helper_task(10, false);
        task.allow_extra_args = false;
        let result = run(
            &store,
            "fixed",
            &task,
            &["extra".into()],
            false,
            RelayMode::Silent,
        )
        .await;
        assert!(matches!(result, Err(AgentError::InvalidArgument(_))));
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn timeout_kills_process_tree() {
        let _guard = CHILD_LOCK.lock().await;
        set_child_mode("tree-parent");
        let marker = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("task-grandchild-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        unsafe {
            std::env::set_var("EVERYDAY_TASK_CHILD_FILE", &marker);
        }
        let path = db_path("tree");
        let _ = std::fs::remove_file(&path);
        let store = TaskStore::open_path(&path).await.unwrap();
        let record = run(
            &store,
            "tree",
            &helper_task(1, true),
            &[],
            false,
            RelayMode::Silent,
        )
        .await
        .unwrap();
        assert_eq!(record.status, "timeout");
        assert_eq!(mirrored_exit_code(&record), 124);
        tokio::time::sleep(Duration::from_millis(4300)).await;
        assert!(
            !std::path::Path::new(&marker).exists(),
            "grandchild survived timeout"
        );
        clear_child_mode();
        drop(store);
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(marker);
    }

    #[tokio::test]
    async fn capture_is_truncated_per_stream() {
        let _guard = CHILD_LOCK.lock().await;
        set_child_mode("large");
        let path = db_path("large");
        let _ = std::fs::remove_file(&path);
        let store = TaskStore::open_path(&path).await.unwrap();
        let record = run(
            &store,
            "large",
            &helper_task(10, true),
            &[],
            false,
            RelayMode::Silent,
        )
        .await
        .unwrap();
        let stdout = record.stdout.unwrap();
        assert!(stdout.contains("[truncated at 65536 bytes]"));
        assert!(stdout.len() < 66 * 1024);
        clear_child_mode();
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
