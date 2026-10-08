//! Запись журнала поколений вокруг применения и отката непринятого (`reset`). Замер 8.3.27.2074: поколение основной
//! конфигурации меняет загрузка, а не применение; у расширения `ibcmd` читает загруженное, а
//! Конфигуратор — применённое, и его поколение меняет именно применение. Поэтому по токену
//! непринятое не узнать (его помнит признак `applied: false`), и запись не угадывает:
//! поколение до применения сверяется с записью, а после него читается тем же инструментом,
//! что записал её, и запись переносится на ответ
//! (`INV.USE-CASES.A-LOAD-WITHOUT-APPLY-IS-REMEMBERED-AS-UNAPPLIED`).

use std::path::Path;

use crate::domain::source_set::SourceSetContext;
use crate::domain::status::GenerationRecordFate;
use crate::support::error::AppError;
use crate::use_cases::agent_session::{ApplyMark, GenerationLedger, GenerationRecord};

/// После шага `step` — применения, перед которым поколение совпало с записью, или отката
/// непринятого: ответ инструмента записи становится записью — с прежней операцией и снятым
/// признаком «не применено»: после обоих шагов основная конфигурация равна конфигурации базы
/// данных. Без ответа запись стирается: прежний токен мог описывать базу до шага, и следующая
/// отправка приняла бы свой шаг за чужую правку. Строка — для ответа. Отмена, замеченная при
/// чтении, стирает запись и останавливает шаг: её называет отказ вместе со стёртой записью,
/// как после загрузки (`INV.USE-CASES.A-LOAD-RECORDS-ITS-GENERATION-OR-ERASES-THE-RECORD`).
pub(crate) fn carry_record(
    step: &str,
    set: &SourceSetContext,
    work_path: &Path,
    record: &GenerationRecord,
    after: Result<Option<String>, AppError>,
) -> Result<(GenerationRecordFate, Option<String>), AppError> {
    let Some(ledger) = GenerationLedger::of(set, work_path) else {
        return Ok((GenerationRecordFate::Unchecked, None));
    };
    let after = match after {
        Ok(after) => after,
        Err(error) if error.cancellation().is_some() => {
            let (_, note) = erase(
                step,
                set,
                &ledger,
                &format!("is not known after the {step}"),
            );
            return Err(match note {
                Some(note) => error.with_context(note),
                None => error,
            });
        }
        Err(error) => {
            tracing::debug!(%error, step, "the generation after the step is not known");
            None
        }
    };
    let Some(after) = after else {
        return Ok(erase(
            step,
            set,
            &ledger,
            &format!("is not known after the {step}"),
        ));
    };
    Ok(
        match ledger.record_as(record.tool, &after, record.after, ApplyMark::Applied) {
            Ok(()) => (GenerationRecordFate::Recorded, None),
            Err(error) => erase(
                step,
                set,
                &ledger,
                &format!("was not recorded after the {step} ({error})"),
            ),
        },
    )
}

/// Стирает запись набора после шага `step`, у которого нет ответа инструмента записи.
pub(crate) fn erase_record(
    step: &str,
    set: &SourceSetContext,
    work_path: &Path,
    why: &str,
) -> (GenerationRecordFate, Option<String>) {
    match GenerationLedger::of(set, work_path) {
        Some(ledger) => erase(step, set, &ledger, why),
        None => (GenerationRecordFate::Unchecked, None),
    }
}

fn erase(
    step: &str,
    set: &SourceSetContext,
    ledger: &GenerationLedger,
    why: &str,
) -> (GenerationRecordFate, Option<String>) {
    let name = set.name();
    match ledger.forget() {
        Ok(_) => (
            GenerationRecordFate::Erased,
            Some(format!(
                "the configuration generation of source-set '{name}' {why}: its record is erased, so the next push does not check whether the infobase moved ahead"
            )),
        ),
        // Стереть не вышло: запись осталась прежней, и ответ называет это предупреждением.
        Err(error) => (
            GenerationRecordFate::Unerased,
            Some(format!(
                "the configuration generation of source-set '{name}' {why}, and its record was not erased: {error}; the next push may take this {step} for a change made elsewhere"
            )),
        ),
    }
}
