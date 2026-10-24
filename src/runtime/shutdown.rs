use tokio::sync::watch;
use tracing::info;

/// Cloneable shutdown signal. Fires once on Ctrl-C or SIGTERM (or when
/// triggered programmatically); every clone observes it.
#[derive(Clone)]
pub struct Shutdown {
    tx: watch::Sender<bool>,
    rx: watch::Receiver<bool>,
}

impl Shutdown {
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(false);
        Self { tx, rx }
    }

    /// Create a shutdown signal wired to Ctrl-C and SIGTERM.
    pub fn from_os_signals() -> Self {
        let shutdown = Self::new();
        let trigger = shutdown.clone();
        tokio::spawn(async move {
            wait_for_os_signal().await;
            info!("Shutdown signal received");
            trigger.trigger();
        });
        shutdown
    }

    pub fn trigger(&self) {
        let _ = self.tx.send(true);
    }

    pub fn is_triggered(&self) -> bool {
        *self.rx.borrow()
    }

    /// Resolves once shutdown has been triggered.
    pub async fn wait(&self) {
        let mut rx = self.rx.clone();
        // An error means every sender is gone, which can only happen after
        // the owning Shutdown is dropped; treat that as shutdown too.
        let _ = rx.wait_for(|triggered| *triggered).await;
    }
}

impl Default for Shutdown {
    fn default() -> Self {
        Self::new()
    }
}

async fn wait_for_os_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use super::Shutdown;
    use std::time::Duration;

    #[tokio::test]
    async fn every_clone_observes_trigger() {
        let shutdown = Shutdown::new();
        let observer = shutdown.clone();
        assert!(!observer.is_triggered());

        let waiter = tokio::spawn(async move { observer.wait().await });
        shutdown.trigger();
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("waiter must finish after trigger")
            .unwrap();
        assert!(shutdown.is_triggered());
    }
}
