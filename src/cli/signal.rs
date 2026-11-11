use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use tokio_util::sync::CancellationToken;

use crate::use_cases::context::OperatorInsistence;

/// Строка журнала, которой раннер отмечает, что прерывание оператора дошло до команды:
/// отмена уже выставлена, когда она записана.
const OPERATOR_INTERRUPT_RECEIVED: &str = "operator interrupt received; the command is cancelled";

/// Строка журнала второго сигнала: раннер перестаёт ждать то, что снять не может.
const OPERATOR_INSISTS: &str =
    "second operator interrupt received; the runner stops waiting for work it cannot stop";

/// Сколько после первого сигнала следующий ещё считается тем же нажатием.
const SECOND_SIGNAL_GAP: std::time::Duration = std::time::Duration::from_secs(1);

/// Lifetime guard that forwards CLI termination signals into a shared cancellation token.
///
/// Первый `Ctrl+C` или `SIGTERM` отменяет команду. Второй выставляет второй сигнал
/// ([`OperatorInsistence`]): раннер перестаёт ждать команду чужого агента, которую снять
/// не может, и отвечает, что бросил её.
pub struct CliSignalGuard {
    stop_tx: Option<mpsc::Sender<()>>,
    join: Option<JoinHandle<()>>,
    _insistence: OperatorInsistence,
}

impl CliSignalGuard {
    /// Starts a background listener that maps `Ctrl+C` and `SIGTERM` into the token.
    pub fn install(cancellation: CancellationToken) -> Self {
        let insistence = CancellationToken::new();
        let slot = OperatorInsistence::enter(insistence.clone());
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
                let mut stopped = tokio::task::spawn_blocking(move || {
                    let _ = stop_rx.recv();
                });
                let mut first_at: Option<std::time::Instant> = None;
                for (token, line) in [
                    (cancellation, OPERATOR_INTERRUPT_RECEIVED),
                    (insistence, OPERATOR_INSISTS),
                ] {
                    loop {
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
                            _ = tokio::signal::ctrl_c() => {}
                            _ = termination => {}
                            _ = &mut stopped => return,
                        }
                        // Одно нажатие бывает двумя сигналами — `SIGINT` группе и `SIGTERM` от
                        // обёртки: вторым считается только сигнал не раньше `SECOND_SIGNAL_GAP`.
                        match first_at {
                            Some(first) if first.elapsed() < SECOND_SIGNAL_GAP => {}
                            _ => break,
                        }
                    }
                    first_at.get_or_insert_with(std::time::Instant::now);
                    token.cancel();
                    tracing::info!("{}", line);
                }
            });
        });

        Self {
            stop_tx: Some(stop_tx),
            join: Some(join),
            _insistence: slot,
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
