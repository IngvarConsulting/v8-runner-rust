//! `make` без набора: пакет каждого набора проекта в каталог `--output`.
//!
//! Наборы обходятся одним порядком [`SourceSetInventory::ordered_source_sets`]: сперва пакеты
//! конфигурации — основная конфигурация и расширения, — затем внешние обработки и отчёты.
//! До первой сборки цели всех пакетов сверяются с каталогами наборов и `workPath`
//! ([`SourceSetInventory::check_package_targets`]). Каждый набор собирается тем же
//! сценарием, что `make <SET>`, в [`package_in_directory`]; отказ набора останавливает обход.
//! Временная база у обхода одна ([`super::MakeSession`]): основная конфигурация попадает в
//! неё один раз, расширения ложатся поверх, и после обхода база убирается.

use std::time::Instant;

use crate::config::model::AppConfig;
use crate::domain::artifacts::MakeAllResult;
use crate::use_cases::context::{CommandName, ExecutionContext};
use crate::use_cases::request::{ArtifactsModeRequest, ArtifactsRequest, MakeAllRequest};
use crate::use_cases::result::{stamp_dispatch, UseCaseResult};
use crate::use_cases::set_walk;
use crate::use_cases::source_inventory::{package_in_directory, SourceSetInventory};

pub fn execute_all(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &MakeAllRequest,
) -> UseCaseResult<MakeAllResult> {
    stamp_dispatch(run_all(context, config, request), context.work())
}

fn run_all(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &MakeAllRequest,
) -> UseCaseResult<MakeAllResult> {
    let started = Instant::now();
    let mut result = MakeAllResult {
        ok: false,
        provider_dispatched: false,
        output_path: request.output_directory.clone(),
        sets: Vec::new(),
        duration_ms: 0,
        message: None,
    };
    let inventory = SourceSetInventory::new(config);
    let sets = inventory.ordered_source_sets();
    if let Err(error) =
        inventory.check_package_targets(CommandName::Artifacts, &request.output_directory, &sets)
    {
        return Err(set_walk::fail(error, result, started));
    }
    let mut session = super::MakeSession::new(config);
    let walked = walk(context, config, request, &sets, &mut session, &mut result);
    // База обхода убирается и после отказа; неудачную уборку называет последний набор.
    let warning = session.close();
    if let Some(last) = result.sets.last_mut() {
        super::note_cleanup_warning(last, warning);
    }
    if let Err(error) = walked {
        return Err(set_walk::fail(error, result, started));
    }
    result.ok = true;
    Ok(set_walk::finish(result, started))
}

fn walk(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &MakeAllRequest,
    sets: &[&crate::config::model::SourceSetConfig],
    session: &mut super::MakeSession,
    result: &mut MakeAllResult,
) -> Result<(), crate::use_cases::result::UseCaseError> {
    for source_set in sets.iter().copied() {
        let mode = ArtifactsModeRequest::for_purpose(source_set.purpose);
        let set_request = ArtifactsRequest {
            execution: ArtifactsRequest::default_execution(mode),
            mode,
            output_path: package_in_directory(&request.output_directory, source_set)
                .display()
                .to_string(),
            source_set: Some(source_set.name.clone()),
            extension: None,
            dry_run: request.dry_run,
            // Каталог внешнего набора назван именем набора: точка в имени суффиксом не
            // становится.
            output_is_directory: matches!(
                mode,
                ArtifactsModeRequest::ExternalDataProcessorEpf
                    | ArtifactsModeRequest::ExternalReportErf
            ),
        };
        set_walk::collect_set(
            &mut result.sets,
            super::execute_in(context, config, &set_request, session),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::execute_all;
    use crate::config::model::{
        AppConfig, InfobaseConfig, PlatformToolConfig, SourceFormat, SourceSetConfig,
        SourceSetPurpose, TestsConfig, ToolsConfig,
    };
    use crate::use_cases::context::{CommandName, ExecutionContext};
    use crate::use_cases::request::MakeAllRequest;
    use crate::use_cases::result::UseCaseErrorKind;
    use tokio_util::sync::CancellationToken;

    /// Отмена между наборами останавливает обход на безопасной точке следующего набора:
    /// ответ называет этот набор, а за ним ничего не собирается.
    #[test]
    fn a_cancellation_stops_the_walk_at_the_next_set() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("base");
        for set in ["cf", "ext"] {
            std::fs::create_dir_all(base.join(set)).expect("set dir");
        }
        // Конфигуратор не запускается: отмена приходит раньше.
        let platform = dir.path().join("1cv8");
        std::fs::write(&platform, "#!/bin/sh\nexit 1\n").expect("platform");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&platform, std::fs::Permissions::from_mode(0o755))
                .expect("permissions");
        }
        let config = AppConfig {
            base_path: base.clone(),
            work_path: dir.path().join("work"),
            format: SourceFormat::Designer,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: InfobaseConfig::file("File=/tmp/ib"),
            infobases: Default::default(),
            infobase_name: None,
            source_sets: vec![
                SourceSetConfig {
                    name: "main".to_owned(),
                    purpose: SourceSetPurpose::Configuration,
                    path: "cf".into(),
                },
                SourceSetConfig {
                    name: "ext".to_owned(),
                    purpose: SourceSetPurpose::Extension,
                    path: "ext".into(),
                },
            ],
            tools: ToolsConfig {
                platform: PlatformToolConfig {
                    path: Some(platform),
                    strict: false,
                    version: None,
                },
                ..ToolsConfig::default()
            },
            mcp: Default::default(),
            tests: TestsConfig::default(),
        };
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let context = ExecutionContext::cli(CommandName::Artifacts).with_cancellation(cancellation);
        let request = MakeAllRequest {
            output_directory: dir.path().join("out"),
            dry_run: false,
        };

        let failure = execute_all(&context, &config, &request).expect_err("cancelled");
        assert!(
            matches!(failure.error.kind(), UseCaseErrorKind::Cancelled(_)),
            "{}",
            failure.error
        );
        let payload = failure.payload.expect("walk payload");
        assert!(!payload.ok);
        assert_eq!(payload.sets.len(), 1, "{payload:?}");
        assert_eq!(payload.sets[0].source_set.as_deref(), Some("main"));
        assert!(!dir.path().join("out/main.cf").exists());
        assert!(!dir.path().join("out/ext.cfe").exists());
    }
}
