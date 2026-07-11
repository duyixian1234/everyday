//! mail 模块的 IMAP session 连接池。
//!
//! 固定大小 M=4（ADR 0010），所有 session 共享同一份 keyring 密码。
//! 跨文件夹 sync 时通过 `tokio::Semaphore` 控制并发上限为 4。
//!
//! `acquire()` 返回 `PoolGuard`，内部 take 一个空闲 session；
//! `Drop` 时归还（除非调用 `invalidate()` 标记为 dirty）。
//!
//! 启动时同步建满 4 个 session —— 减少首次 list 的 4×TLS 握手延迟叠加。

use std::collections::VecDeque;
use std::sync::Arc;

use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

use crate::config::MailAccount;
use crate::error::{AgentError, Result};
use crate::modules::email::{ImapSession, imap_connect};

/// 池大小。ADR 0010：写死，无 flag / config 暴露。
pub const POOL_SIZE: usize = 4;

/// IMAP session 池（cheap-clone，内部 `Arc`）。
#[derive(Clone)]
pub struct Pool {
    inner: Arc<PoolInner>,
}

struct PoolInner {
    /// 空闲 session 队列。`acquire` pop_front，`Drop` push_back。
    sessions: Mutex<VecDeque<ImapSession>>,
    /// 并发上限（默认 `POOL_SIZE`）。`Arc` 包裹以便 `acquire_owned`。
    semaphore: Arc<Semaphore>,
    /// 重建脏 session 用的账号元数据 + 密码（来自 keyring）。
    #[allow(dead_code)]
    account: MailAccount,
    #[allow(dead_code)]
    password: String,
}

impl Pool {
    /// 建池：启动时同步建立 `POOL_SIZE` 个 IMAP session。
    pub async fn new(account: MailAccount, password: String) -> Result<Self> {
        let mut sessions = VecDeque::with_capacity(POOL_SIZE);
        for _ in 0..POOL_SIZE {
            sessions.push_back(imap_connect(&account, &password).await?);
        }
        Ok(Self {
            inner: Arc::new(PoolInner {
                sessions: Mutex::new(sessions),
                semaphore: Arc::new(Semaphore::new(POOL_SIZE)),
                account,
                password,
            }),
        })
    }

    /// 获取一个 session 的独占所有权（`PoolGuard`）。
    ///
    /// - 阻塞至信号量有空位
    /// - 池空时报错（防御性，正常不会触发）
    pub async fn acquire(&self) -> Result<PoolGuard> {
        let permit = Arc::clone(&self.inner.semaphore)
            .acquire_owned()
            .await
            .map_err(|e| AgentError::Other(format!("acquire pool semaphore: {e}")))?;
        let mut sessions = self.inner.sessions.lock().await;
        let session = sessions.pop_front().ok_or_else(|| {
            AgentError::Other("pool exhausted: semaphore/signaled mismatch".into())
        })?;
        Ok(PoolGuard {
            pool: Arc::clone(&self.inner),
            permit,
            session: Some(session),
        })
    }
}

/// `Pool::acquire` 返回的 session guard。
///
/// - `Drop` 时自动归还 session 到池
/// - `invalidate()` 标记 session 为 dirty，drop 时不归还（防止坏 session 被复用）
pub struct PoolGuard {
    pool: Arc<PoolInner>,
    /// 持有 permit 限制并发；guard drop 时 permit 自动释放。
    /// 字段从未显式读取，但 drop 语义负责释放 semaphore。
    #[allow(dead_code)]
    permit: OwnedSemaphorePermit,
    /// `Some` = 持有 session；`None` = 已通过 `invalidate` 消费。
    session: Option<ImapSession>,
}

impl PoolGuard {
    /// 取得内部 session 的可变引用以执行 IMAP 命令。
    ///
    /// 已被 `invalidate()` 消费时返回错误而非 panic（生产路径禁止 unwrap）。
    pub fn session(&mut self) -> Result<&mut ImapSession> {
        self.session
            .as_mut()
            .ok_or_else(|| AgentError::Other("pool guard session already consumed".into()))
    }

    /// 标记 session 为 dirty（命令失败后调用），drop 时不归还。
    pub fn invalidate(mut self) {
        self.session.take();
    }
}

impl Drop for PoolGuard {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            // 归还 session 到空闲队列
            let pool = Arc::clone(&self.pool);
            tokio::spawn(async move {
                let mut sessions = pool.sessions.lock().await;
                sessions.push_back(session);
            });
        }
        // permit 在 OwnedSemaphorePermit drop 时自动释放 → 并发槽 +1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_is_4() {
        // 编译期常量的稳定性测试：ADR 0010 写死 M=4
        assert_eq!(POOL_SIZE, 4);
    }

    #[tokio::test]
    async fn session_after_invalidate_returns_error_not_panic() {
        // 验证修复：之前 session() 在 invalidate 之后会 .expect() panic。
        // 现版本必须返回 Result，且 Ok/Err 路径都不 panic。
        // 这里构造一个空的 PoolGuard 字段等价结构（绕过真实 IMAP 连接）。
        let permit = Arc::new(Semaphore::new(1))
            .acquire_owned()
            .await
            .expect("semaphore acquire");
        let mut guard = PoolGuard {
            pool: Arc::new(PoolInner {
                sessions: Mutex::new(VecDeque::new()),
                semaphore: Arc::new(Semaphore::new(1)),
                account: MailAccount {
                    name: String::new(),
                    imap_host: String::new(),
                    imap_port: 993,
                    smtp_host: String::new(),
                    smtp_port: 587,
                    username: String::new(),
                    tls: true,
                },
                password: String::new(),
            }),
            permit,
            session: None,
        };

        let result = guard.session();
        assert!(result.is_err(), "session() after invalidate must error");
    }

    // 注：完整 Pool 行为测试需要 mock IMAP server，超出单测范围（CI 跳过网络）。
    // 见 docs/adr/0010 §Consequences — 真实环境验证在集成测试中跑。
}
