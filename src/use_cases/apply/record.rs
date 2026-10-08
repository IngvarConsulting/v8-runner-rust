//! Запись журнала поколений вокруг применения. Применение может сменить токен поколения, а
//! может и не сменить (замера нет), поэтому запись не угадывает: поколение до применения
//! сверяется с записью, а после него читается тем же инструментом, что записал её, и
//! запись переносится на ответ (`INV.USE-CASES.A-LOAD-WITHOUT-APPLY-IS-REMEMBERED-AS-UNAPPLIED`).

use std::path::Path;

use crate::domain::apply::ApplyGeneration;
use crate::domain::source_set::SourceSetContext;
use crate::use_cases::agent_session::{GenerationLedger, GenerationRecord};

/// После применения, перед которым поколение совпало с записью: ответ инструмента записи
/// становится записью — с прежней операцией и снятым признаком «не применено». Без ответа
/// запись стирается: прежний токен мог описывать базу до применения, и следующая отправка
/// приняла бы своё применение за чужую правку. Строка — для ответа.
pub(crate) fn carry_after_apply(
    set: &SourceSetContext,
    work_path: &Path,
    record: &GenerationRecord,
    after: Option<&str>,
) -> (ApplyGeneration, Option<String>) {
    let Some(ledger) = GenerationLedger::of(set, work_path) else {
        return (ApplyGeneration::Unchecked, None);
    };
    let Some(after) = after else {
        return erase(set, &ledger, "is not known after the apply");
    };
    match ledger.record_as(record.tool, after, record.after, true) {
        Ok(()) => (ApplyGeneration::Recorded, None),
        Err(error) => erase(
            set,
            &ledger,
            &format!("was not recorded after the apply ({error})"),
        ),
    }
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
        Err(error) => (
            ApplyGeneration::Kept,
            Some(format!(
                "the configuration generation of source-set '{name}' {why}, and its record was not erased: {error}; the next push may take this apply for a change made elsewhere"
            )),
        ),
    }
}
