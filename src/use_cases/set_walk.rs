//! Общее у обходов наборов — `pull --all`, `make` и `download` без набора: ответ каждого
//! набора копится в `sets` его формой, первый отказ останавливает обход, а ответ обхода
//! закрывается одинаково — длительностью и, у отказа, его текстом.
//!
//! Отмена между наборами отдельной проверки не требует: сценарий набора спрашивает её на
//! своей первой безопасной точке, и ответ набора называет прерывание.

use std::time::Instant;

use crate::platform::process::WorkGiven;
use crate::use_cases::result::{UseCaseError, UseCaseFailure, UseCaseResult};

/// Ответ обхода, который закрывает этот модуль.
pub(crate) trait WalkReport {
    /// Длительность обхода и, у отказа, его текст.
    fn close(&mut self, duration_ms: u64, message: Option<String>);
}

macro_rules! walk_report {
    ($($ty:ty),* $(,)?) => {
        $(impl WalkReport for $ty {
            fn close(&mut self, duration_ms: u64, message: Option<String>) {
                self.duration_ms = duration_ms;
                if message.is_some() {
                    self.message = message;
                }
            }
        })*
    };
}

walk_report!(
    crate::domain::dump::PullAllResult,
    crate::domain::artifacts::MakeAllResult,
    crate::domain::infobase_export::DownloadAllResult,
);

/// Кладёт ответ набора в обход: удачу — в `sets`; у отказа — то, что набор успел, если успел,
/// и ошибку вызывающему, чтобы тот остановил обход.
pub(crate) fn collect_set<T>(
    sets: &mut Vec<T>,
    outcome: UseCaseResult<T>,
) -> Result<(), UseCaseError> {
    match outcome {
        Ok(done) => {
            sets.push(done);
            Ok(())
        }
        Err(failure) => {
            sets.extend(failure.payload);
            Err(failure.error)
        }
    }
}

/// Удачный конец обхода: длительность.
pub(crate) fn finish<R: WalkReport>(mut result: R, started: Instant) -> R {
    result.close(elapsed_ms(started), None);
    result
}

/// Отказ обхода: ответ несёт сделанное до него и называет причину.
pub(crate) fn fail<R: WalkReport>(
    error: impl Into<UseCaseError>,
    mut result: R,
    started: Instant,
) -> UseCaseFailure<R> {
    let error = error.into();
    result.close(elapsed_ms(started), Some(error.to_string()));
    UseCaseFailure::with_payload(error, result)
}

/// Отказ там, где исполнитель, может быть, уже получил работу (чтение состава базы): после
/// работы — формой обхода с причиной, до работы — общей формой отказа.
pub(crate) fn fail_after_possible_work<R: WalkReport>(
    error: impl Into<UseCaseError>,
    mut result: R,
    started: Instant,
    work: &WorkGiven,
) -> UseCaseFailure<R> {
    let error = error.into();
    let message = error.to_string();
    UseCaseFailure::after_possible_work(error, work, move || {
        result.close(elapsed_ms(started), Some(message));
        result
    })
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::collect_set;
    use crate::use_cases::result::{UseCaseError, UseCaseErrorKind, UseCaseFailure};

    /// Отказ набора кладёт в обход то, что набор успел, и отдаёт ошибку; удача — сам ответ.
    #[test]
    fn a_refused_set_keeps_its_payload_and_stops_the_walk() {
        let mut sets = Vec::new();
        assert!(collect_set(&mut sets, Ok(1)).is_ok());
        let error = UseCaseError::new(UseCaseErrorKind::Validation, "refused");
        let stopped = collect_set(&mut sets, Err(UseCaseFailure::with_payload(error, 2)));
        assert_eq!(stopped.expect_err("stops").message(), "refused");
        let error = UseCaseError::new(UseCaseErrorKind::Validation, "before");
        assert!(collect_set(&mut sets, Err(UseCaseFailure::without_payload(error))).is_err());
        assert_eq!(sets, [1, 2]);
    }
}
