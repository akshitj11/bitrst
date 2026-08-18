//! Shutdown signalling helpers for the CLI and tests.

use std::future::Future;

use tokio::sync::watch;

/// Waits for Ctrl-C (all platforms) or SIGTERM (Unix).
pub async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// A persistent shutdown trigger that cannot lose wakeups like [`tokio::sync::Notify`].
#[derive(Clone)]
pub struct ShutdownTrigger {
    tx: watch::Sender<bool>,
}

impl ShutdownTrigger {
    /// Creates a new trigger and a future that completes when signalled.
    pub fn pair() -> (Self, impl Future<Output = ()> + Send) {
        let (tx, mut rx) = watch::channel(false);
        let wait = async move {
            let _ = rx.wait_for(|value| *value).await;
        };
        (Self { tx }, wait)
    }

    /// Signals shutdown. Safe to call before or after the waiter starts.
    pub fn signal(&self) {
        let _ = self.tx.send(true);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::ShutdownTrigger;

    #[tokio::test]
    async fn shutdown_trigger_survives_early_signal() {
        for _ in 0..32 {
            let (trigger, wait) = ShutdownTrigger::pair();
            trigger.signal();
            tokio::time::timeout(Duration::from_millis(100), wait)
                .await
                .expect("early signal must not be lost");
        }
    }
}
