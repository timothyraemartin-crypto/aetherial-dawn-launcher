//! Work that must finish but must never hold Play up, such as the staff
//! report upload (the staff service can rate-limit for 90+ seconds).

use std::future::Future;

/// Runs `work` on its own task and returns at once. The work keeps going
/// after the caller returns, so nothing is dropped; only the wait is gone.
/// Needs a running tokio runtime (a Tauri command has one).
pub fn detach<F>(work: F) -> tokio::task::JoinHandle<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(work)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test(start_paused = true)]
    async fn slow_work_does_not_hold_the_caller_and_still_finishes() {
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        let start = tokio::time::Instant::now();
        // A report upload that waits out a 90 s rate limit.
        let handle = detach(async move {
            tokio::time::sleep(Duration::from_secs(100)).await;
            flag.store(true, Ordering::SeqCst);
        });
        assert!(start.elapsed() < Duration::from_secs(1), "caller waited for the work");
        assert!(!done.load(Ordering::SeqCst));
        handle.await.unwrap();
        assert!(done.load(Ordering::SeqCst), "the work was dropped instead of finishing");
        assert!(start.elapsed() >= Duration::from_secs(100));
    }
}
