//! Запись журнала поколений вокруг применения. Замер 8.3.27.2074: поколение основной
//! конфигурации меняет загрузка, а не применение; у расширения `ibcmd` читает загруженное, а
//! Конфигуратор — применённое, и его поколение меняет именно применение. Поэтому по токену
//! непринятое не узнать (его помнит признак `applied: false`), и запись не угадывает:
//! поколение до применения сверяется с записью, а после него читается тем же инструментом,
//! что записал её, и запись переносится на ответ
//! (`INV.USE-CASES.A-LOAD-WITHOUT-APPLY-IS-REMEMBERED-AS-UNAPPLIED`).

use std::path::Path;

use crate::domain::apply::ApplyGeneration;
use crate::domain::source_set::SourceSetContext;
use crate::support::error::AppError;
use crate::use_cases::agent_session::{GenerationLedger, GenerationRecord};

/// После применения, перед которым поколение совпало с записью: ответ инструмента записи
/// становится записью — с прежней операцией и снятым признаком «не применено». Без ответа
/// запись стирается: прежний токен мог описывать базу до применения, и следующая отправка
/// приняла бы своё применение за чужую правку. Строка — для ответа. Отмена, замеченная при
/// чтении, стирает запись и останавливает шаг: её называет отказ вместе со стёртой записью,
/// как после загрузки (`INV.USE-CASES.A-LOAD-RECORDS-ITS-GENERATION-OR-ERASES-THE-RECORD`).
pub(crate) fn carry_after_apply(
    set: &SourceSetContext,
    work_path: &Path,
    record: &GenerationRecord,
    after: Result<Option<String>, AppError>,
) -> Result<(ApplyGeneration, Option<String>), AppError> {
    let Some(ledger) = GenerationLedger::of(set, work_path) else {
        return Ok((ApplyGeneration::Unchecked, None));
    };
    let after = match after {
        Ok(after) => after,
        Err(error) if error.cancellation().is_some() => {
            let (_, note) = erase(set, &ledger, "is not known after the apply");
            return Err(match note {
                Some(note) => error.with_context(note),
                None => error,
            });
        }
        Err(error) => {
            tracing::debug!(%error, "the generation after the apply is not known");
            None
        }
    };
    let Some(after) = after else {
        return Ok(erase(set, &ledger, "is not known after the apply"));
    };
    Ok(
        match ledger.record_as(record.tool, &after, record.after, true) {
            Ok(()) => (ApplyGeneration::Recorded, None),
            Err(error) => erase(
                set,
                &ledger,
                &format!("was not recorded after the apply ({error})"),
            ),
        },
    )
}

/// Стирает запись набора после применения, у которого нет ответа инструмента записи.
pub(crate) fn erase_after_apply(
    set: &SourceSetContext,
    work_path: &Path,
    why: &str,
) -> (ApplyGeneration, Option<String>) {
    match GenerationLedger::of(set, work_path) {
        Some(ledger) => erase(set, &ledger, why),
        None => (ApplyGeneration::Unchecked, None),
    }
}

fn erase(
    set: &SourceSetContext,
    ledger: &GenerationLedger,
    why: &str,
) -> (ApplyGeneration, Option<String>) {
    let name = set.name();
    match ledger.forget() {
        Ok(_) => (
            ApplyGeneration::Erased,
            Some(format!(
                "the configuration generation of source-set '{name}' {why}: its record is erased, so the next push does not check whether the infobase moved ahead"
            )),
        ),
        // Стереть не вышло: запись осталась прежней, и ответ называет это предупреждением —
        // значение `kept` занято за базой, ушедшей от записи.
        Err(error) => (
            ApplyGeneration::Erased,
            Some(format!(
                "the configuration generation of source-set '{name}' {why}, and its record was not erased: {error}; the next push may take this apply for a change made elsewhere"
            )),
        ),
    }
}
