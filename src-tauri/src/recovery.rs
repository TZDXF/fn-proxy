use crate::{
    error::{error, AppError, Result},
    text::Text,
};
use futures_util::future::BoxFuture;
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex as StdMutex, RwLock,
    },
    time::Duration,
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

pub const MAX_ATTEMPTS: usize = 3;
pub const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(60);
const REQUEST_WAIT: Duration = Duration::from_secs(70);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryReason {
    SessionUnavailable,
    HeartbeatFailed,
    EntryExpired,
    PeriodicRefresh,
    DomainInventoryFailed,
}
impl RecoveryReason {
    fn label(self) -> &'static str {
        match self {
            Self::SessionUnavailable => "session_unavailable",
            Self::HeartbeatFailed => "heartbeat_failed",
            Self::EntryExpired => "entry_expired",
            Self::PeriodicRefresh => "periodic_refresh",
            Self::DomainInventoryFailed => "domain_inventory_failed",
        }
    }
    pub fn for_inventory_error(error: &AppError) -> Option<Self> {
        // Neither a partial source permission failure nor malformed registry data
        // alone proves logout. Probe authentication only when all sources failed.
        (error.text().code == "inventory.sourcesUnavailable").then_some(Self::DomainInventoryFailed)
    }
    pub fn refresh_first(self) -> bool {
        matches!(
            self,
            Self::EntryExpired | Self::PeriodicRefresh | Self::DomainInventoryFailed
        )
    }
}
pub type RecoveryHandler =
    Arc<dyn Fn(RecoveryReason) -> BoxFuture<'static, Result<()>> + Send + Sync>;
pub type DiagnosticHandler = Arc<dyn Fn(&str, Text) + Send + Sync>;

/// One repair per connection, shared by all HTTP/WS listeners and the heartbeat monitor.
/// A failed episode is latched until an explicit successful connection resets it, preventing
/// request bursts from submitting passwords again after NAS rejection or exhausted retries.
#[derive(Default)]
pub struct Recovery {
    generation: AtomicU64,
    gate: Mutex<()>,
    handler: RwLock<Option<RecoveryHandler>>,
    diagnostic: RwLock<Option<DiagnosticHandler>>,
    failure: StdMutex<Option<(u64, Text)>>,
    last_repair: StdMutex<Option<tokio::time::Instant>>,
}
impl Recovery {
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
    pub fn configure(&self, handler: RecoveryHandler, diagnostic: DiagnosticHandler) {
        *self.handler.write().unwrap() = Some(handler);
        *self.diagnostic.write().unwrap() = Some(diagnostic);
        *self.failure.lock().unwrap() = None;
        *self.last_repair.lock().unwrap() = None;
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
    pub fn clear(&self) {
        *self.handler.write().unwrap() = None;
        *self.failure.lock().unwrap() = None;
        *self.last_repair.lock().unwrap() = None;
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
    pub fn log(&self, level: &str, text: Text) {
        let observer = self.diagnostic.read().unwrap().clone();
        if let Some(observer) = observer {
            observer(level, text);
        }
    }
    pub async fn recover(
        self: &Arc<Self>,
        reason: RecoveryReason,
        observed: u64,
        cancel: &CancellationToken,
    ) -> Result<()> {
        self.recover_with_wait(reason, observed, cancel, REQUEST_WAIT)
            .await
    }
    async fn recover_with_wait(
        self: &Arc<Self>,
        reason: RecoveryReason,
        observed: u64,
        cancel: &CancellationToken,
        wait: Duration,
    ) -> Result<()> {
        // A caller's disconnect/timeout must not abort repair shared by other listeners.
        let recovery = self.clone();
        let worker = tokio::spawn(async move { recovery.run(reason, observed).await });
        tokio::select! {
            _ = cancel.cancelled() => Err(error("recovery.cancelled")),
            result = tokio::time::timeout(wait, worker) => match result {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => Err(error("recovery.failed")),
                Err(_) => Err(error("recovery.inProgress")),
            }
        }
    }
    pub async fn maintain(
        self: &Arc<Self>,
        reason: RecoveryReason,
        observed: u64,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let recovery = self.clone();
        let worker = tokio::spawn(async move { recovery.run(reason, observed).await });
        tokio::select! {
            _ = cancel.cancelled() => Err(error("recovery.cancelled")),
            result = worker => result.unwrap_or_else(|_| Err(error("recovery.failed"))),
        }
    }
    async fn run(&self, reason: RecoveryReason, observed: u64) -> Result<()> {
        let _guard = self.gate.lock().await;
        let generation = self.generation();
        if let Some((failed_generation, text)) = self.failure.lock().unwrap().as_ref() {
            if *failed_generation == generation {
                return Err(AppError::Code(text.clone()));
            }
        }
        if generation != observed {
            return Ok(());
        }
        if reason == RecoveryReason::EntryExpired
            && self
                .last_repair
                .lock()
                .unwrap()
                .is_some_and(|at| at.elapsed() < Duration::from_secs(10))
        {
            return Err(error("recovery.cooldown"));
        }
        let handler = self
            .handler
            .read()
            .unwrap()
            .clone()
            .ok_or_else(|| error("recovery.unavailable"))?;
        self.log(
            "info",
            Text::with(
                "logs.recoveryStarted",
                [("reason", reason.label().to_owned())],
            ),
        );
        let result = handler(reason).await;
        // Do not let a cancelled old monitor corrupt a newly connected session's coordinator.
        if self.generation() != generation {
            return result;
        }
        match &result {
            Ok(()) => {
                *self.last_repair.lock().unwrap() = Some(tokio::time::Instant::now());
                self.generation.fetch_add(1, Ordering::AcqRel);
            }
            Err(e) if e.text().code != "recovery.cancelled" => {
                *self.failure.lock().unwrap() = Some((generation, e.text()));
            }
            _ => {}
        }
        result
    }
}

pub async fn retry<T, F, Fut>(
    cancel: &CancellationToken,
    mut operation: F,
    diagnostic: &(dyn Fn(&str, Text) + Sync),
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    retry_with_limits(
        cancel,
        &mut operation,
        diagnostic,
        ATTEMPT_TIMEOUT,
        [Duration::from_secs(2), Duration::from_secs(5)],
    )
    .await
}
async fn retry_with_limits<T, F, Fut>(
    cancel: &CancellationToken,
    operation: &mut F,
    diagnostic: &(dyn Fn(&str, Text) + Sync),
    timeout: Duration,
    backoff: [Duration; 2],
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    for attempt in 1..=MAX_ATTEMPTS {
        if cancel.is_cancelled() {
            return Err(error("recovery.cancelled"));
        }
        diagnostic(
            "info",
            Text::with(
                "logs.reconnectAttempt",
                [
                    ("attempt", attempt.to_string()),
                    ("max", MAX_ATTEMPTS.to_string()),
                ],
            ),
        );
        let result = tokio::select! {
            _ = cancel.cancelled() => return Err(error("recovery.cancelled")),
            result = tokio::time::timeout(timeout, operation()) => match result {
                Ok(result) => result,
                Err(e) => Err(AppError::from(e).at("reconnect_total")),
            }
        };
        match result {
            Ok(value) => return Ok(value),
            Err(e) => {
                diagnostic("warn", e.diagnostic(attempt));
                if !e.retryable() {
                    diagnostic("error", Text::new("logs.reconnectManualRequired"));
                    return Err(e);
                }
                if attempt == MAX_ATTEMPTS {
                    diagnostic(
                        "error",
                        Text::with(
                            "logs.reconnectExhausted",
                            [("max", MAX_ATTEMPTS.to_string())],
                        ),
                    );
                    return Err(e);
                }
                let delay = backoff[attempt - 1];
                diagnostic(
                    "warn",
                    Text::with(
                        "logs.reconnectBackoff",
                        [("seconds", delay.as_secs().to_string())],
                    ),
                );
                tokio::select! {
                    _ = cancel.cancelled() => return Err(error("recovery.cancelled")),
                    _ = tokio::time::sleep(delay) => {},
                }
            }
        }
    }
    unreachable!()
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use tokio::sync::Notify;

    #[test]
    fn unavailable_registries_probe_authentication_before_relogin() {
        let reason =
            RecoveryReason::for_inventory_error(&error("inventory.sourcesUnavailable")).unwrap();
        assert_eq!(reason, RecoveryReason::DomainInventoryFailed);
        assert!(reason.refresh_first());
        assert_eq!(reason.label(), "domain_inventory_failed");
        assert!(RecoveryReason::for_inventory_error(&error("inventory.listMissing")).is_none());
    }

    fn transient() -> AppError {
        AppError::Io(std::io::Error::from(std::io::ErrorKind::ConnectionReset)).at("resolve")
    }
    async fn fast_retry<T, F, Fut>(
        cancel: &CancellationToken,
        mut operation: F,
        events: &StdMutex<Vec<Text>>,
    ) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        retry_with_limits(
            cancel,
            &mut operation,
            &|_, text| events.lock().unwrap().push(text),
            Duration::from_millis(30),
            [Duration::from_millis(1); 2],
        )
        .await
    }
    #[tokio::test]
    async fn transient_failures_retry_with_backoff_then_succeed() {
        let attempts = AtomicUsize::new(0);
        let events = StdMutex::new(Vec::new());
        let value = fast_retry(
            &CancellationToken::new(),
            || async {
                if attempts.fetch_add(1, Ordering::SeqCst) < 2 {
                    Err(transient())
                } else {
                    Ok(42)
                }
            },
            &events,
        )
        .await
        .unwrap();
        assert_eq!(value, 42);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        assert_eq!(
            events
                .lock()
                .unwrap()
                .iter()
                .filter(|t| t.code == "logs.reconnectBackoff")
                .count(),
            2
        );
    }
    #[tokio::test]
    async fn network_failures_stop_after_three_attempts() {
        let attempts = AtomicUsize::new(0);
        let events = StdMutex::new(Vec::new());
        let result: Result<()> = fast_retry(
            &CancellationToken::new(),
            || async {
                attempts.fetch_add(1, Ordering::SeqCst);
                Err(transient())
            },
            &events,
        )
        .await;
        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), MAX_ATTEMPTS);
        assert!(events
            .lock()
            .unwrap()
            .iter()
            .any(|t| t.code == "logs.reconnectExhausted"));
    }
    #[tokio::test]
    async fn account_rejection_and_two_factor_never_retry_passwords() {
        for code in [
            "auth.nasRejected",
            "auth.tfaRequired",
            "auth.tfaBindingRequired",
        ] {
            let attempts = AtomicUsize::new(0);
            let events = StdMutex::new(Vec::new());
            let result: Result<()> = fast_retry(
                &CancellationToken::new(),
                || async {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    Err(error(code).at("user_login"))
                },
                &events,
            )
            .await;
            assert!(result.is_err());
            assert_eq!(attempts.load(Ordering::SeqCst), 1);
            assert!(events
                .lock()
                .unwrap()
                .iter()
                .any(|t| t.code == "logs.reconnectManualRequired"));
        }
    }
    #[tokio::test]
    async fn timeouts_are_bounded_and_retryable() {
        let attempts = AtomicUsize::new(0);
        let events = StdMutex::new(Vec::new());
        let result: Result<()> = fast_retry(
            &CancellationToken::new(),
            || async {
                attempts.fetch_add(1, Ordering::SeqCst);
                std::future::pending().await
            },
            &events,
        )
        .await;
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        assert_eq!(result.unwrap_err().category(), "timeout");
    }
    #[tokio::test]
    async fn cancellation_interrupts_network_and_backoff() {
        for during_backoff in [false, true] {
            let cancel = CancellationToken::new();
            let events = StdMutex::new(Vec::new());
            let cancel_copy = cancel.clone();
            let result: Result<()> = fast_retry(
                &cancel,
                || async {
                    if during_backoff {
                        cancel_copy.cancel();
                        Err(transient())
                    } else {
                        cancel_copy.cancel();
                        std::future::pending().await
                    }
                },
                &events,
            )
            .await;
            assert_eq!(result.unwrap_err().text().code, "recovery.cancelled");
        }
    }
    #[tokio::test]
    async fn parallel_requests_and_monitor_share_one_repair() {
        let recovery = Arc::new(Recovery::default());
        let attempts = Arc::new(AtomicUsize::new(0));
        let calls = attempts.clone();
        recovery.configure(
            Arc::new(move |_| {
                let calls = calls.clone();
                Box::pin(async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    Ok(())
                })
            }),
            Arc::new(|_, _| {}),
        );
        let observed = recovery.generation();
        let cancel = CancellationToken::new();
        let results = futures_util::future::join_all(
            (0..20).map(|_| recovery.recover(RecoveryReason::EntryExpired, observed, &cancel)),
        )
        .await;
        assert!(results.into_iter().all(|r| r.is_ok()));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn failed_episode_is_latched_until_manual_connection_resets_it() {
        let recovery = Arc::new(Recovery::default());
        let attempts = Arc::new(AtomicUsize::new(0));
        let calls = attempts.clone();
        recovery.configure(
            Arc::new(move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Err(error("auth.nasRejected")) })
            }),
            Arc::new(|_, _| {}),
        );
        let observed = recovery.generation();
        let cancel = CancellationToken::new();
        for _ in 0..5 {
            assert!(recovery
                .recover(RecoveryReason::SessionUnavailable, observed, &cancel)
                .await
                .is_err());
        }
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        recovery.configure(
            Arc::new(|_| Box::pin(async { Ok(()) })),
            Arc::new(|_, _| {}),
        );
        recovery
            .recover(
                RecoveryReason::SessionUnavailable,
                recovery.generation(),
                &cancel,
            )
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn cancellation_of_one_request_does_not_abort_shared_repair() {
        let recovery = Arc::new(Recovery::default());
        let started = Arc::new(Notify::new());
        let finish = Arc::new(Notify::new());
        let started_copy = started.clone();
        let finish_copy = finish.clone();
        recovery.configure(
            Arc::new(move |_| {
                let started = started_copy.clone();
                let finish = finish_copy.clone();
                Box::pin(async move {
                    started.notify_one();
                    finish.notified().await;
                    Ok(())
                })
            }),
            Arc::new(|_, _| {}),
        );
        let observed = recovery.generation();
        let cancel = CancellationToken::new();
        let first = {
            let r = recovery.clone();
            let c = cancel.clone();
            tokio::spawn(async move { r.recover(RecoveryReason::EntryExpired, observed, &c).await })
        };
        started.notified().await;
        cancel.cancel();
        assert_eq!(
            first.await.unwrap().unwrap_err().text().code,
            "recovery.cancelled"
        );
        finish.notify_one();
        recovery
            .recover(
                RecoveryReason::EntryExpired,
                observed,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn stale_cancelled_worker_cannot_poison_new_manual_connection() {
        let recovery = Arc::new(Recovery::default());
        let started = Arc::new(Notify::new());
        let finish = Arc::new(Notify::new());
        let sc = started.clone();
        let fc = finish.clone();
        recovery.configure(
            Arc::new(move |_| {
                let s = sc.clone();
                let f = fc.clone();
                Box::pin(async move {
                    s.notify_one();
                    f.notified().await;
                    Err(error("auth.nasRejected"))
                })
            }),
            Arc::new(|_, _| {}),
        );
        let observed = recovery.generation();
        let pending = {
            let r = recovery.clone();
            tokio::spawn(async move {
                r.recover(
                    RecoveryReason::HeartbeatFailed,
                    observed,
                    &CancellationToken::new(),
                )
                .await
            })
        };
        started.notified().await;
        recovery.clear();
        recovery.configure(
            Arc::new(|_| Box::pin(async { Ok(()) })),
            Arc::new(|_, _| {}),
        );
        finish.notify_one();
        assert!(pending.await.unwrap().is_err());
        recovery
            .recover(
                RecoveryReason::SessionUnavailable,
                recovery.generation(),
                &CancellationToken::new(),
            )
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn request_wait_timeout_does_not_latch_failure_or_abort_background_repair() {
        let recovery = Arc::new(Recovery::default());
        let started = Arc::new(Notify::new());
        let finish = Arc::new(Notify::new());
        let sc = started.clone();
        let fc = finish.clone();
        recovery.configure(
            Arc::new(move |_| {
                let s = sc.clone();
                let f = fc.clone();
                Box::pin(async move {
                    s.notify_one();
                    f.notified().await;
                    Ok(())
                })
            }),
            Arc::new(|_, _| {}),
        );
        let observed = recovery.generation();
        let waiting = {
            let r = recovery.clone();
            tokio::spawn(async move {
                r.recover_with_wait(
                    RecoveryReason::EntryExpired,
                    observed,
                    &CancellationToken::new(),
                    Duration::from_millis(10),
                )
                .await
            })
        };
        started.notified().await;
        assert_eq!(
            waiting.await.unwrap().unwrap_err().text().code,
            "recovery.inProgress"
        );
        assert!(recovery.failure.lock().unwrap().is_none());
        finish.notify_one();
        recovery
            .maintain(
                RecoveryReason::HeartbeatFailed,
                observed,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_ne!(recovery.generation(), observed);
    }
    #[tokio::test]
    async fn repeated_expired_pages_have_a_success_cooldown() {
        let recovery = Arc::new(Recovery::default());
        let attempts = Arc::new(AtomicUsize::new(0));
        let calls = attempts.clone();
        recovery.configure(
            Arc::new(move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Ok(()) })
            }),
            Arc::new(|_, _| {}),
        );
        let cancel = CancellationToken::new();
        recovery
            .recover(RecoveryReason::EntryExpired, recovery.generation(), &cancel)
            .await
            .unwrap();
        assert_eq!(
            recovery
                .recover(RecoveryReason::EntryExpired, recovery.generation(), &cancel)
                .await
                .unwrap_err()
                .text()
                .code,
            "recovery.cooldown"
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }
}
