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
/// Прерывание оператора — `Ctrl+C`, `SIGTERM` на Unix, `CTRL_BREAK` на Windows. Закрытие
/// окна консоли (`CTRL_CLOSE`) не слушается: Windows даёт на него секунды и снимает процесс,
/// дождаться агента всё равно нельзя. Первое прерывание отменяет команду. Второе выставляет второй сигнал
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
                // Третий путь прерывания: `SIGTERM` на Unix и `CTRL_BREAK` на Windows — его
                // шлёт тот, кто запустил раннер в своей группе процессов: `CTRL_C` в чужую группу
                // не доставить, а по умолчанию `CTRL_BREAK` обрывает процесс без ответа.
                #[cfg(unix)]
                let mut term =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
                #[cfg(windows)]
                let mut term = tokio::signal::windows::ctrl_break().ok();
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
                        #[cfg(windows)]
                        let termination = async {
                            match term.as_mut() {
                                Some(term) => {
                                    let _ = term.recv().await;
                                }
                                None => std::future::pending::<()>().await,
                            }
                        };
                        #[cfg(not(any(unix, windows)))]
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
                    let first = first_at.is_none();
                    first_at.get_or_insert_with(std::time::Instant::now);
                    token.cancel();
                    tracing::info!("{}", line);
                    // Первый сигнал — событие `cancel` хода команды (`CTR.WIRE.PROGRESS-EVENTS`):
                    // прямо в поток, и тогда, когда журнал команды ещё не заведён.
                    if first {
                        crate::output::events::cancel(
                            crate::domain::progress::CancelReason::Cancelled,
                        );
                    }
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
