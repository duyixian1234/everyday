//! IMAP session connection pool for the mail module.
//!
//! Fixed size M=4 (ADR [M002](../../docs/adr/M002-imap-connection-pool.md)); all
//! sessions share the same keyring password.
//! The idle queue is the sole source of checkout capacity, and `Notify` wakes
//! waiters after a session is synchronously returned.
//!
//! Sessions are built eagerly (all 4) at startup to avoid stacking 4× TLS
//! handshake latency on the first list.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

use crate::config::MailAccount;
use crate::error::{AgentError, Result};
use crate::modules::email::{ImapSession, imap_connect};

/// Pool size. ADR [M002](../../docs/adr/M002-imap-connection-pool.md): hard-coded, no flag / config exposure.
pub const POOL_SIZE: usize = 4;

struct SessionState<T> {
    sessions: VecDeque<T>,
    checked_out: usize,
    rebuilding: usize,
    last_rebuild_error: Option<String>,
}

struct SessionQueue<T> {
    state: Mutex<SessionState<T>>,
    available: Notify,
}

impl<T> SessionQueue<T> {
    fn new(sessions: VecDeque<T>) -> Self {
        Self {
            state: Mutex::new(SessionState {
                sessions,
                checked_out: 0,
                rebuilding: 0,
                last_rebuild_error: None,
            }),
            available: Notify::new(),
        }
    }

    async fn acquire(self: &Arc<Self>) -> Result<SessionLease<T>> {
        loop {
            let notified = self.available.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            {
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| AgentError::Other("mail pool state lock poisoned".into()))?;
                if let Some(session) = state.sessions.pop_front() {
                    state.checked_out += 1;
                    return Ok(SessionLease {
                        queue: Arc::clone(self),
                        session: Some(session),
                    });
                }
                if state.checked_out == 0 && state.rebuilding == 0 {
                    let detail = state
                        .last_rebuild_error
                        .as_deref()
                        .unwrap_or("no live sessions");
                    return Err(AgentError::Other(format!(
                        "mail pool has no available sessions: {detail}"
                    )));
                }
            }

            notified.await;
        }
    }

    fn start_rebuild(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.checked_out = state.checked_out.saturating_sub(1);
            state.rebuilding += 1;
        }
    }

    fn finish_rebuild(&self, session: Option<T>, error: Option<String>) {
        if let Ok(mut state) = self.state.lock() {
            state.rebuilding = state.rebuilding.saturating_sub(1);
            if let Some(session) = session {
                state.sessions.push_back(session);
                state.last_rebuild_error = None;
            } else if let Some(error) = error {
                state.last_rebuild_error = Some(error);
            }
        }
        self.available.notify_waiters();
    }
}

struct SessionLease<T> {
    queue: Arc<SessionQueue<T>>,
    session: Option<T>,
}

impl<T> SessionLease<T> {
    fn session(&mut self) -> Option<&mut T> {
        self.session.as_mut()
    }

    fn invalidate(mut self) {
        if self.session.take().is_some() {
            self.queue.start_rebuild();
        }
    }
}

impl<T> Drop for SessionLease<T> {
    fn drop(&mut self) {
        let Some(session) = self.session.take() else {
            return;
        };
        if let Ok(mut state) = self.queue.state.lock() {
            state.checked_out = state.checked_out.saturating_sub(1);
            state.sessions.push_back(session);
            drop(state);
            self.queue.available.notify_one();
        }
    }
}

/// IMAP session pool (cheap-clone, backed by `Arc`).
#[derive(Clone)]
pub struct Pool {
    inner: Arc<PoolInner>,
}

struct PoolInner {
    queue: Arc<SessionQueue<ImapSession>>,
    account: MailAccount,
    password: String,
}

impl Pool {
    /// Build the pool: create `POOL_SIZE` IMAP sessions eagerly at startup.
    pub async fn new(account: MailAccount, password: String) -> Result<Self> {
        let mut sessions = VecDeque::with_capacity(POOL_SIZE);
        for _ in 0..POOL_SIZE {
            sessions.push_back(imap_connect(&account, &password).await?);
        }
        Ok(Self {
            inner: Arc::new(PoolInner {
                queue: Arc::new(SessionQueue::new(sessions)),
                account,
                password,
            }),
        })
    }

    /// Acquire exclusive ownership of a session (`PoolGuard`), waiting when all
    /// live sessions are checked out or being rebuilt.
    pub async fn acquire(&self) -> Result<PoolGuard> {
        Ok(PoolGuard {
            pool: Arc::clone(&self.inner),
            lease: Some(self.inner.queue.acquire().await?),
        })
    }
}

/// Session guard returned by `Pool::acquire`.
///
/// Returns the session to the pool on `Drop`. An invalidated session is replaced
/// in the background before it becomes available again.
pub struct PoolGuard {
    pool: Arc<PoolInner>,
    lease: Option<SessionLease<ImapSession>>,
}

impl PoolGuard {
    /// Borrow the inner session mutably to run IMAP commands.
    pub fn session(&mut self) -> Result<&mut ImapSession> {
        self.lease
            .as_mut()
            .and_then(SessionLease::session)
            .ok_or_else(|| AgentError::Other("pool guard session already consumed".into()))
    }

    /// Mark the session dirty and rebuild its replacement in the background.
    pub fn invalidate(mut self) {
        let Some(lease) = self.lease.take() else {
            return;
        };
        lease.invalidate();

        let pool = Arc::clone(&self.pool);
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    match imap_connect(&pool.account, &pool.password).await {
                        Ok(session) => pool.queue.finish_rebuild(Some(session), None),
                        Err(error) => {
                            tracing::warn!(
                                account = %pool.account.name,
                                error = %error,
                                "failed to rebuild invalidated IMAP session"
                            );
                            pool.queue.finish_rebuild(None, Some(error.to_string()));
                        }
                    }
                });
            }
            Err(error) => {
                pool.queue.finish_rebuild(
                    None,
                    Some(format!(
                        "runtime unavailable while rebuilding session: {error}"
                    )),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[test]
    fn capacity_is_4() {
        assert_eq!(POOL_SIZE, 4);
    }

    #[tokio::test]
    async fn six_waiters_share_four_sessions_without_exhaustion() {
        let queue = Arc::new(SessionQueue::new((0..POOL_SIZE).collect()));
        let completed = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();

        for _ in 0..6 {
            let queue = Arc::clone(&queue);
            let completed = Arc::clone(&completed);
            tasks.push(tokio::spawn(async move {
                let _lease = queue.acquire().await.expect("session checkout");
                tokio::task::yield_now().await;
                completed.fetch_add(1, Ordering::SeqCst);
            }));
        }

        tokio::time::timeout(Duration::from_secs(1), futures::future::join_all(tasks))
            .await
            .expect("all waiters should complete");
        assert_eq!(completed.load(Ordering::SeqCst), 6);
        assert_eq!(
            queue.state.lock().expect("queue lock").sessions.len(),
            POOL_SIZE
        );
    }

    #[tokio::test]
    async fn invalidated_session_is_unavailable_until_rebuilt() {
        let queue = Arc::new(SessionQueue::new(VecDeque::from([1])));
        queue
            .acquire()
            .await
            .expect("session checkout")
            .invalidate();

        let waiting_queue = Arc::clone(&queue);
        let waiter = tokio::spawn(async move { waiting_queue.acquire().await });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        queue.finish_rebuild(Some(2), None);
        let mut lease = waiter.await.expect("waiter task").expect("rebuilt session");
        assert_eq!(lease.session(), Some(&mut 2));
    }
}
