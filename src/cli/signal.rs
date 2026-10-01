use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use tokio_util::sync::CancellationToken;

/// Строка журнала, которой раннер отмечает, что прерывание оператора дошло до команды:
/// отмена уже выставлена, когда она записана.
const OPERATOR_INTERRUPT_RECEIVED: &str = "operator interrupt received; the command is cancelled";

/// Lifetime guard that forwards CLI termination signals into a shared cancellation token.
pub struct CliSignalGuard {
    stop_tx: Option<mpsc::Sender<()>>,
    join: Option<JoinHandle<()>>,
}

impl CliSignalGuard {
    /// Starts a background listener that maps `Ctrl+C` and `SIGTERM` into the token.
    pub fn install(cancellation: CancellationToken) -> Self {
        let (stop_tx, stop_rx) = mpsc::channel();
        let join = thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(_) => return,
            };

            runtime.block_on(async move {
                #[cfg(unix)]
                let mut term =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
                #[cfg(unix)]
                let termination = async {
                    match term.as_mut() {
                        Some(term) => {
                            let _ = term.recv().await;
                        }
                        None => std::future::pending::<()>().await,
                    }
                };
                #[cfg(not(unix))]
                let termination = std::future::pending::<()>();

                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {
                        cancellation.cancel();
                        tracing::info!("{}", OPERATOR_INTERRUPT_RECEIVED);
                    }
                    _ = tokio::task::spawn_blocking(move || {
                        let _ = stop_rx.recv();
                    }) => {}
                    _ = termination => {
                        cancellation.cancel();
                        tracing::info!("{}", OPERATOR_INTERRUPT_RECEIVED);
                    }
                }
            });
        });

        Self {
            stop_tx: Some(stop_tx),
            join: Some(join),
        }
    }
}

impl Drop for CliSignalGuard {
    fn drop(&mut self) {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
