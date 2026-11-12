//! Выгрузка поверх каталога набора через стадию (#646).
//!
//! Файла версий в каталоге набора нет: он лежит в памяти раннера
//! ([`RunnerVersionFile`]). Поэтому выгрузка по изменившемуся, как и полная поверх каталога,
//! идёт в пустой каталог стадии `workPath/stage` (на томе набора), а не в сам каталог: по копии
//! описи платформа пишет туда только изменённое и новую опись. Раннер переносит удаления по
//! двум описям, кладёт написанное поверх каталога — всё, кроме описи, — и делает новую опись
//! своей копией. Копия меняется последней: сбой раньше оставляет прежнюю, и следующая
//! выгрузка по ней выгрузит больше, а не меньше.

use std::collections::HashSet;

use super::*;
use crate::use_cases::version_file::RunnerVersionFile;

/// Подкаталог стадии, куда пишет платформа: его ещё нет, когда она стартует. Выгрузка по
/// чужой описи (`-configDumpInfoForChanges`, `--config-dump-info-for-changes`, `--base`) замерена
/// в несуществующий каталог.
const OUTPUT_DIR: &str = "out";

/// Что выгрузка поверх каталога отдаёт платформе: каталог, куда писать, и копию описи, по
/// которой считать изменённое (`None` — выгрузка полная).
pub(super) struct DumpInto<'a> {
    pub(super) output: &'a Path,
    pub(super) description: Option<&'a Path>,
}

/// Выгрузка поверх каталога набора через стадию: `dump` пишет в [`DumpInto::output`], по копии
/// описи раннера, если выгрузка по изменившемуся. Прежняя опись разбирается, пока работает
/// платформа. После удачи — удаления по двум описям, перенос поверх каталога и новая копия;
/// возвращает то, что вернул `dump`, и предупреждения переноса. Сбой платформы или переноса
/// убирает стадию и оставляет копию прежней.
pub(super) fn dump_over_directory<T>(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    how: &OverDirectory,
    version: Option<&RunnerVersionFile>,
    dump: impl FnOnce(DumpInto<'_>) -> Result<T, AppError>,
) -> Result<(T, Option<String>), AppError> {
    let description = match how {
        OverDirectory::ByVersionFile => Some(
            version
                .map(RunnerVersionFile::path)
                .ok_or_else(|| {
                    AppError::Runtime(format!(
                        "the dump by changes of source-set '{}' has no runner copy of {VERSION_FILE_NAME}",
                        resolved.source_set_name
                    ))
                })?,
        ),
        OverDirectory::Whole(_) => None,
    };
    let stage = StagedPublication::prepare_dir_in(
        &resolved.stage_root,
        &resolved.platform_target_path,
        &resolved.platform_target_identity,
        DUMP_STAGE_PREFIX,
    )?;
    let output = stage.staging_path().join(OUTPUT_DIR);
    let (dumped, before) = beside(
        "dump-description",
        || description.and_then(|path| description_elements(path, "before")),
        || {
            dump(DumpInto {
                output: &output,
                description,
            })
        },
    );
    let dumped = dumped.map_err(|error| stage.cleanup_failure(error))?;
    let message = land(
        context, config, resolved, &stage, &output, how, before, version,
    )?;
    Ok((dumped, message))
}

/// Узнал ли раннер, какие элементы пропали из базы.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Removals {
    /// Удаления перенесены или их не было; у полной выгрузки их не переносят вовсе.
    Known,
    /// Одна из описей не разобрана или новая пуста при непустой прежней.
    Unknown,
}

/// Посадка написанного: удаления по двум описям, перенос поверх каталога, новая копия, уборка
/// стадии. `how` — шла ли выгрузка по копии описи; `before` — элементы прежней описи,
/// `None` — она не разобрана. Пока удаления не известны, копия не меняется.
#[allow(clippy::too_many_arguments)]
fn land(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    stage: &StagedPublication,
    output: &Path,
    how: &OverDirectory,
    before: Option<HashSet<String>>,
    version: Option<&RunnerVersionFile>,
) -> Result<Option<String>, AppError> {
    if let Some(error) = interruption_before_publish(context, "dump landing") {
        return Err(stage.cleanup_failure(error));
    }
    let description = output.join(VERSION_FILE_NAME);
    let removals = match (how, before) {
        (OverDirectory::Whole(_), _) => Removals::Known,
        (OverDirectory::ByVersionFile, None) => Removals::Unknown,
        (OverDirectory::ByVersionFile, Some(before)) => {
            let inventory = SourceSetInventory::new(config);
            match inventory.designer_context(&resolved.source_set_name) {
                Some(set) => remove_dropped_elements(context, config, set, &description, &before)
                    .map_err(|error| stage.cleanup_failure(error))?,
                None => Removals::Known,
            }
        }
    };
    stage.lay_over_dir(
        context,
        "failed to lay the staged dump over the directory",
        output,
        VERSION_FILE_NAME,
    )?;
    let copy_warning = match version {
        // Какие элементы пропали, не узнать: новая опись копией не становится, иначе следующая
        // выгрузка уже не увидела бы их удаления. Прежняя копия старше дерева и безопасна.
        // Отметка держит и отправку от свежей описи, пока выгрузка не запишет копию.
        Some(version) if removals == Removals::Unknown => merge_optional_messages(
            Some(format!(
                "the removals of the dump of source-set '{}' are not known: a description was not read; the runner's copy was not updated, so the next pull dumps the same changes again; run {} to replace the directory with the base and the copy with its description",
                resolved.source_set_name,
                context.advised_pull_force(&resolved.source_set_name)
            )),
            version.mark_removals_unknown(),
        ),
        Some(version) if description.is_file() => version.record_from(&description),
        Some(_) => Some(format!(
            "the dump of source-set '{}' left no {VERSION_FILE_NAME}; the runner's copy was not updated, so the next pull may dump more than changed",
            resolved.source_set_name
        )),
        None => None,
    };
    debug!(
        target = %resolved.platform_target_path.display(),
        "the staged dump was laid over the directory"
    );
    Ok(merge_optional_messages(copy_warning, stage.discard()))
}

/// Элементы описи `path` — прежней (`side` = `before`) или новой; `None`, если её нет или она
/// не разобрана: по такой описи не сказать, что пропало. Удаления по ней не переносятся, а
/// новая опись копией не становится.
fn description_elements(path: &Path, side: &str) -> Option<HashSet<String>> {
    crate::platform::dump_format::read_dump_elements(path)
        .inspect_err(|error| {
            debug!(
                path = %path.display(),
                side,
                %error,
                "the dump description was not read; no removal is carried over"
            );
        })
        .ok()
}

/// Удаления выгрузки по изменившемуся (`INV.USE-CASES.THE-DUMP-MODE-IS-FORECAST-IN-THE-SAME-COMMAND`):
/// файл, чей владелец был в прежней описи `before` и пропал из новой в `description`, удаляется
/// из каталога набора — в его области обхода, без ссылок, до переноса написанного и до того, как
/// раннер запомнит записанное. Новой описи нет, она не разобрана или пуста при непустой прежней —
/// не удаляется ничего: такая опись не описывает базу. Пропавших элементов нет — каталог не
/// перечисляется. Неудачное удаление — отказ с перечнем оставшегося: каталог тогда не равен
/// базе, и молчать об этом нельзя.
fn remove_dropped_elements(
    context: &ExecutionContext,
    config: &AppConfig,
    set: &SourceSetContext,
    description: &Path,
    before: &HashSet<String>,
) -> Result<Removals, AppError> {
    let Some(after) = description_elements(description, "after") else {
        return Ok(Removals::Unknown);
    };
    if after.is_empty() && !before.is_empty() {
        debug!(
            path = %description.display(),
            "the new dump description names no element; no removal is carried over"
        );
        return Ok(Removals::Unknown);
    }
    let caseless_after: HashSet<String> = after.iter().map(|name| name.to_lowercase()).collect();
    if before
        .iter()
        .all(|name| after.contains(name) || caseless_after.contains(&name.to_lowercase()))
    {
        return Ok(Removals::Known);
    }
    let dropped = crate::change_detection::analyzer::files_of_dropped_elements(
        set,
        &config.work_path,
        None,
        before,
        &after,
        &|| crate::use_cases::interruption::interruption_pending(context),
    )
    .map_err(|error| {
        AppError::Runtime(format!(
            "the dump of source-set '{}' is in the stage, but the files of removed elements were not found: {error}",
            set.name()
        ))
    })?;
    remove_files(set, &dropped).map(|()| Removals::Known)
}

/// Удаляет файлы `files` из каталога набора `set` и каталоги, опустевшие от этого; непустой
/// каталог не удаляется. Что удалить не удалось, называет отказ.
fn remove_files(set: &SourceSetContext, files: &[PathBuf]) -> Result<(), AppError> {
    let mut left = Vec::new();
    for file in files {
        if let Err(error) = std::fs::remove_file(file) {
            if error.kind() != std::io::ErrorKind::NotFound {
                left.push(format!("'{}': {error}", file.display()));
            }
            continue;
        }
        let mut dir = file.parent();
        while let Some(current) = dir.filter(|current| *current != set.path()) {
            if std::fs::remove_dir(current).is_err() {
                break;
            }
            dir = current.parent();
        }
    }
    if left.is_empty() {
        return Ok(());
    }
    Err(AppError::Runtime(format!(
        "the dump of source-set '{}' is in the stage, but {} file(s) of removed elements were not deleted: {}",
        set.name(),
        left.len(),
        left.join("; ")
    )))
}

#[cfg(test)]
mod tests {
    use super::remove_files;
    use crate::domain::source_set::SourceSetContext;

    /// Удаление, которое не удалось, — отказ, называющий оставшееся; удалённое не мешает
    /// назвать остальное, а опустевший каталог уходит вслед.
    #[test]
    fn a_failed_removal_names_what_is_left() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("src");
        let gone = root.join("Catalogs").join("Gone.xml");
        let stuck = root.join("Catalogs").join("Stuck");
        std::fs::create_dir_all(stuck.join("Ext")).expect("a directory where a file was");
        std::fs::write(&gone, "gone").expect("file");
        let empty = root.join("Documents").join("Empty").join("Ext");
        std::fs::create_dir_all(&empty).expect("dirs");
        let lone = empty.join("ObjectModule.bsl");
        std::fs::write(&lone, "lone").expect("file");
        let set = SourceSetContext::new("main", root.clone(), "designer-main");

        let error = remove_files(&set, &[gone.clone(), stuck.clone(), lone])
            .expect_err("a directory is not removed as a file");

        let message = error.to_string();
        assert!(message.contains("1 file(s)"), "{message}");
        assert!(message.contains(&stuck.display().to_string()), "{message}");
        assert!(!gone.exists());
        assert!(
            !root.join("Documents").exists(),
            "emptied directories go too"
        );
        assert!(
            root.join("Catalogs").is_dir(),
            "a directory with files stays"
        );
    }
}
