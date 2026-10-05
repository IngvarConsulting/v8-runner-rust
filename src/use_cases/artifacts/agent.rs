//! `make` через агентский shell Конфигуратора: одна сессия на команду.
//!
//! Файловые параметры (`--file=`, `--ext-file=`) агент не разрешает через символическую
//! ссылку — только каталоги `--dir=` (замер 15.09.2026). Поэтому исходники внешней
//! обработки копируются в `make/<run>/`, результат появляется там же и переносится в
//! стадию публикации семейства.

use std::path::{Path, PathBuf};

use crate::config::model::AppConfig;
use crate::domain::artifact::{ArtifactKind, ArtifactRef, ArtifactSet};
use crate::domain::artifacts::ArtifactBuildMode;
use crate::platform::result::PlatformCommandResult;
use crate::support::error::AppError;
use crate::support::fs::{write_temp_dir_metadata, TempDirKind};
use crate::use_cases::agent_session::{
    argument, collect_file, make_output_dir, run_command, run_id, stage_copy_dir, tidy,
    transcript_log, with_session,
};
use crate::use_cases::context::ExecutionContext;
use crate::use_cases::dump_config::verify_external_dump_descriptor;
use crate::use_cases::interruption::interruption_before_safe_point;
use crate::use_cases::progress::log_live_stage;
use crate::use_cases::staged_publication::StagedPublication;

use super::{
    ensure_platform_success, external_descriptors, publication_message, refusal_before_publication,
    sanitize_file_stem, PublicationOutcome, ResolvedArtifactsTarget, ARTIFACTS_BACKUP_PREFIX,
    ARTIFACT_ROLE_PACKAGE_FILE, ARTIFACT_ROLE_PLATFORM_LOG, ARTIFACT_ROLE_STAGE_FILE,
};

type ExportOutcome = Result<
    (PlatformCommandResult, ArtifactSet, PublicationOutcome),
    (AppError, ArtifactSet, Option<PathBuf>),
>;

pub(super) fn run_agent_export(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedArtifactsTarget,
    v8: Option<&Path>,
) -> ExportOutcome {
    if matches!(
        resolved.mode,
        ArtifactBuildMode::ExternalDataProcessorEpf | ArtifactBuildMode::ExternalReportErf
    ) {
        return run_external_agent_export(context, config, resolved, v8);
    }

    if let Some(error) = interruption_before_safe_point(
        context,
        format!(
            "artifact export for source-set '{}' and output '{}'",
            resolved.source_set_name,
            resolved.output_path.display()
        ),
    ) {
        return Err((error, ArtifactSet::default(), None));
    }

    let publication = StagedPublication::prepare_file(
        &resolved.output_path,
        &resolved.target_identity,
        ".artifacts-stage",
        resolved.mode.file_extension(),
    )
    .map_err(|error| (error, ArtifactSet::default(), None))?;
    let staging_file = publication.staging_path().to_path_buf();
    let cleanup_unmaterialized_stage = |error: AppError| {
        if staging_file.is_file() {
            error
        } else {
            publication.cleanup_failure(error)
        }
    };

    let log = transcript_log(config, &format!("artifacts-{}", resolved.source_set_name)).map_err(
        |error| {
            (
                cleanup_unmaterialized_stage(error),
                ArtifactSet::default(),
                None,
            )
        },
    )?;
    let mut artifacts = ArtifactSet::default();
    artifacts.push(
        ArtifactRef::new(ArtifactKind::PlatformLog, &log).with_role(ARTIFACT_ROLE_PLATFORM_LOG),
    );

    let dump_result = with_session(
        context,
        config,
        v8,
        log.clone(),
        |handle, wait, exchange| {
            let out = format!("make/{}", run_id());
            make_output_dir(handle, exchange, &out)?;
            let relative = format!(
                "{out}/{}.{}",
                sanitize_file_stem(&resolved.source_set_name),
                resolved.mode.file_extension()
            );
            let mut command = format!("config dump-cfg --file={}", argument(&relative));
            if let Some(extension) = resolved.extension.as_deref() {
                command.push_str(&format!(" --extension={}", argument(extension)));
            }
            log_live_stage("make: export", "[агент] exporting artifact package");
            let reply = run_command(handle, &command, wait);
            let staged = reply
                .as_ref()
                .ok()
                .map(|_| collect_file(handle, exchange, &relative, &staging_file));
            tidy(handle, exchange, &out);
            let reply = reply?;
            staged.transpose()?;
            Ok(reply.transcript())
        },
    )
    // Выгрузка критических команд не ведёт, отложенной отмены у её сессии не бывает.
    .map_err(|failure| {
        (
            cleanup_unmaterialized_stage(failure.into_error("artifact export")),
            artifacts.clone(),
            Some(log.clone()),
        )
    })?;
    artifacts.push(
        ArtifactRef::new(
            ArtifactKind::Other("staged_artifact".to_owned()),
            &staging_file,
        )
        .with_role(ARTIFACT_ROLE_STAGE_FILE),
    );

    if let Some(error) = refusal_before_publication(
        context,
        resolved,
        format!(
            "artifact publication for source-set '{}' and output '{}'",
            resolved.source_set_name,
            resolved.output_path.display()
        ),
    ) {
        return Err((error, artifacts, Some(log)));
    }

    let publish_phase = publication
        .publish_file(context, "failed to publish staged artifact")
        .map_err(|error| (error, artifacts.clone(), Some(log.clone())))?;

    let mut published_artifacts = ArtifactSet::default();
    published_artifacts.push(
        ArtifactRef::new(ArtifactKind::Package, &resolved.output_path)
            .with_role(ARTIFACT_ROLE_PACKAGE_FILE),
    );
    Ok((
        dump_result,
        published_artifacts,
        publication_message(context, publish_phase),
    ))
}

/// Внешние обработки и отчёты: исходники копируются в каталог агента, каждый файл
/// собирается `load-external-data-processor-or-report-from-files`, и, как у
/// Конфигуратора, выгружается обратно для сверки вида и имени.
fn run_external_agent_export(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedArtifactsTarget,
    v8: Option<&Path>,
) -> ExportOutcome {
    if let Some(error) = interruption_before_safe_point(
        context,
        format!(
            "external artifact export for source-set '{}' and output '{}'",
            resolved.source_set_name,
            resolved.output_path.display()
        ),
    ) {
        return Err((error, ArtifactSet::default(), None));
    }

    let publication = StagedPublication::prepare_dir(
        &resolved.output_path,
        &resolved.target_identity,
        ".artifacts-stage",
    )
    .map_err(|error| (error, ArtifactSet::default(), None))?;
    let staging_dir = publication.staging_path().to_path_buf();
    let descriptors = external_descriptors(context, config, resolved)
        .map_err(|error| (error, ArtifactSet::default(), None))?;
    let log = transcript_log(config, &format!("artifacts-{}", resolved.source_set_name))
        .map_err(|error| (error, ArtifactSet::default(), None))?;
    let mut artifacts = ArtifactSet::default();
    artifacts.push(
        ArtifactRef::new(ArtifactKind::PlatformLog, &log).with_role(ARTIFACT_ROLE_PLATFORM_LOG),
    );

    let mut staged = Vec::new();
    for descriptor in &descriptors {
        let publish_name = format!(
            "{}.{}",
            sanitize_file_stem(&descriptor.logical_name),
            resolved.mode.file_extension()
        );
        let staging_file = staging_dir.join(&publish_name);
        write_temp_dir_metadata(
            &staging_file,
            TempDirKind::Stage,
            publication.run_id(),
            &resolved.output_path.join(&publish_name),
            &resolved.target_identity,
        )
        .map_err(|error| {
            (
                AppError::Runtime(format!("failed to write staging metadata: {error}")),
                artifacts.clone(),
                None,
            )
        })?;
        staged.push((descriptor, publish_name, staging_file));
    }

    let last_result = with_session(context, config, v8, log.clone(), |handle, wait, exchange| {
        let run = run_id();
        let base = format!("make/{run}");
        let outcome = (|| {
            make_output_dir(handle, exchange, &format!("{base}/out"))?;
            let mut copied: Vec<(PathBuf, String)> = Vec::new();
            for (descriptor, publish_name, staging_file) in &staged {
                let source_link = match copied
                    .iter()
                    .find(|(root, _)| root == &descriptor.root_path)
                {
                    Some((_, link)) => link.clone(),
                    None => {
                        let link = format!("{base}/src-{}", copied.len());
                        stage_copy_dir(handle, exchange, &link, &descriptor.root_path)?;
                        copied.push((descriptor.root_path.clone(), link.clone()));
                        link
                    }
                };
                let descriptor_relative = descriptor
                    .descriptor_xml_path
                    .strip_prefix(&descriptor.root_path)
                    .map_err(|_| {
                        AppError::Runtime(format!(
                            "external descriptor '{}' lies outside its source root '{}'",
                            descriptor.descriptor_xml_path.display(),
                            descriptor.root_path.display()
                        ))
                    })?;
                let xml = format!(
                    "{source_link}/{}",
                    descriptor_relative.display().to_string().replace('\\', "/")
                );
                let out = format!("{base}/out/{publish_name}");
                log_live_stage(
                    "make: external export",
                    "[агент] exporting external artifact package",
                );
                // Загрузка внешней обработки пишет только файл обработки, а база ей нужна
                // лишь для разрешения ссылок (документация `LoadExternalDataProcessorOrReportFromFiles`):
                // фаза не критическая, отмена её снимает, как на пути Конфигуратора.
                run_command(
                    handle,
                    &format!(
                        "config load-external-data-processor-or-report-from-files --file={} --ext-file={}",
                        argument(&xml),
                        argument(&out)
                    ),
                    wait,
                )?;

                log_live_stage(
                    "make: external dump",
                    "[агент] dumping external artifact descriptor",
                );
                let verify = format!("{base}/verify/{}.xml", descriptor.stable_id);
                make_output_dir(handle, exchange, &format!("{base}/verify"))?;
                let reply = run_command(
                    handle,
                    &format!(
                        "config dump-external-data-processor-or-report-to-files --ext-file={} --file={}",
                        argument(&out),
                        argument(&verify)
                    ),
                    wait,
                )?;
                let verify_target = config
                    .work_path
                    .join("external-dump")
                    .join(&resolved.source_set_name)
                    .join(&descriptor.stable_id)
                    .join(format!("{}.xml", descriptor.logical_name));
                if let Some(parent) = verify_target.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| {
                        AppError::Runtime(format!("failed to create external dump dir: {error}"))
                    })?;
                }
                collect_file(handle, exchange, &verify, &verify_target)?;
                verify_external_dump_descriptor(
                    &verify_target,
                    descriptor.artifact_type,
                    &descriptor.logical_name,
                )?;
                collect_file(handle, exchange, &out, staging_file)?;
                if let Some(message) = reply
                    .messages
                    .iter()
                    .filter_map(|message| message.message.as_deref())
                    .find(|text| !text.is_empty())
                {
                    tracing::debug!(message, "external artifact verified through the agent");
                }
            }
            Ok(String::new())
        })();
        tidy(handle, exchange, &base);
        outcome
    })
    // Внешняя обработка собирается некритической загрузкой: отложенной отмены нет и здесь.
    .map_err(|failure| {
        (
            failure.into_error("external artifact export"),
            artifacts.clone(),
            Some(log.clone()),
        )
    })?;
    for (_, _, staging_file) in &staged {
        artifacts.push(
            ArtifactRef::new(ArtifactKind::Package, staging_file)
                .with_role(ARTIFACT_ROLE_STAGE_FILE),
        );
    }
    ensure_platform_success(&resolved.source_set_name, &last_result)
        .map_err(|error| (error, artifacts.clone(), Some(log.clone())))?;

    if let Some(error) = refusal_before_publication(
        context,
        resolved,
        format!(
            "external artifact publication for source-set '{}' and output '{}'",
            resolved.source_set_name,
            resolved.output_path.display()
        ),
    ) {
        return Err((error, artifacts, Some(log)));
    }

    let publish_phase = publication
        .publish_dir(
            context,
            ARTIFACTS_BACKUP_PREFIX,
            "failed to publish staged external directory",
            // Путь вывода — место для порождённого, а не для чьей-то работы.
            crate::use_cases::destruction_guard::DestructionConsent::RunnerOwned,
            &[],
        )
        .map_err(|error| (error, artifacts.clone(), Some(log.clone())))?;

    for (_, publish_name, _) in &staged {
        artifacts.push(
            ArtifactRef::new(
                ArtifactKind::Package,
                resolved.output_path.join(publish_name),
            )
            .with_role(ARTIFACT_ROLE_PACKAGE_FILE),
        );
    }
    Ok((
        last_result,
        artifacts,
        publication_message(context, publish_phase),
    ))
}
