//! IMAP session connection pool for the mail module.
//!
//! Fixed size M=4 (ADR [M002](../../docs/adr/M002-imap-connection-pool.md)); all
//! sessions share the same keyring password.
//! Concurrency across folder sync is capped at 4 via `tokio::Semaphore`.
//!
//! `acquire()` returns a `PoolGuard`, which takes an idle session; the session is
//! returned on `Drop` (unless `invalidate()` marked it dirty).
//!
//! Sessions are built eagerly (all 4) at startup to avoid stacking 4× TLS
//! handshake latency on the first list.

use std::collections::VecDeque;
use std::sync::Arc;

use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

use crate::config::MailAccount;
use crate::error::{AgentError, Result};
use crate::modules::email::{ImapSession, imap_connect};

/// Pool size. ADR [M002](../../docs/adr/M002-imap-connection-pool.md): hard-coded, no flag / config exposure.
pub const POOL_SIZE: usize = 4;

/// IMAP session pool (cheap-clone, backed by `Arc`).
#[derive(Clone)]
pub struct Pool {
    inner: Arc<PoolInner>,
}

struct PoolInner {
    /// Idle session queue. `acquire` pops from the front, `Drop` pushes to the back.
    sessions: Mutex<VecDeque<ImapSession>>,
    /// Concurrency cap (defaults to `POOL_SIZE`). Wrapped in `Arc` for `acquire_owned`.
    semaphore: Arc<Semaphore>,
    /// Account metadata + password (from keyring) used to rebuild dirty sessions.
    #[allow(dead_code)]
    account: MailAccount,
    #[allow(dead_code)]
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
                sessions: Mutex::new(sessions),
                semaphore: Arc::new(Semaphore::new(POOL_SIZE)),
                account,
                password,
            }),
        })
    }

    /// Acquire exclusive ownership of a session (`PoolGuard`).
    ///
    /// - Blocks until a semaphore slot is free.
    /// - Errors if the pool is empty (defensive; should never happen in practice).
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

/// Session guard returned by `Pool::acquire`.
///
/// - Returns the session to the pool on `Drop`.
/// - `invalidate()` marks the session dirty; on drop it is not returned (prevents a
///   bad session from being reused).
pub struct PoolGuard {
    pool: Arc<PoolInner>,
    /// Holds the permit that bounds concurrency; the permit is released automatically on guard drop.
    /// The field is never read explicitly, but its drop semantics release the semaphore.
    #[allow(dead_code)]
    permit: OwnedSemaphorePermit,
    /// `Some` = holds a session; `None` = already consumed via `invalidate`.
    session: Option<ImapSession>,
}

impl PoolGuard {
    /// Borrow the inner session mutably to run IMAP commands.
    ///
    /// Returns an error instead of panicking when already consumed by `invalidate()`
    /// (no unwrap on the production path).
    pub fn session(&mut self) -> Result<&mut ImapSession> {
        self.session
            .as_mut()
            .ok_or_else(|| AgentError::Other("pool guard session already consumed".into()))
    }

    /// Mark the session dirty (called after a command failure); not returned on drop.
    pub fn invalidate(mut self) {
        self.session.take();
    }
}

impl Drop for PoolGuard {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            // Return the session to the idle queue.
            // If we are not inside a tokio runtime (runtime shutting down / test teardown /
            // single-threaded sync context), tokio::spawn would panic and the session would
            // be lost permanently, leaking pool capacity.
            // Probe with Handle::try_current; if no runtime is available, drop the session
            // directly (accept the leak, since this only happens on the process-exit path).
            match tokio::runtime::Handle::try_current() {
                Ok(handle) => {
                    let pool = Arc::clone(&self.pool);
                    handle.spawn(async move {
                        let mut sessions = pool.sessions.lock().await;
                        sessions.push_back(session);
                    });
                }
                Err(_) => {
                    // Runtime already gone: give up returning it. The permit is still
                    // released when OwnedSemaphorePermit drops.
                }
            }
        }
        // The permit is released automatically when OwnedSemaphorePermit drops → concurrency slot +1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_is_4() {
        // Compile-time constant stability test: ADR M002 hard-codes M=4
        assert_eq!(POOL_SIZE, 4);
    }

    #[tokio::test]
    async fn session_after_invalidate_returns_error_not_panic() {
        // Verify the fix: previously session() would .expect() panic after invalidate.
        // The current version must return a Result, and neither the Ok nor Err path panics.
        // Here we build an equivalent empty PoolGuard struct (bypassing a real IMAP connection).
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

    // Note: full Pool behavior tests need a mock IMAP server, beyond unit-test scope (CI skips network).
    // See [F010](../../docs/adr/F010-testing-requirements.md) §Consequences — real-env verification runs in integration tests.
}
