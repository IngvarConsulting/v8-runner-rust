//! `make` без набора: пакет каждого набора проекта в каталог `--output`.
//!
//! Наборы обходятся одним порядком [`SourceSetInventory::ordered_source_sets`]: сперва пакеты
//! конфигурации — основная конфигурация и расширения, — затем внешние обработки и отчёты.
//! Каждый набор собирается тем же сценарием, что `make <SET>`, в
//! [`package_in_directory`]; отказ набора останавливает обход.

use std::time::Instant;

use crate::config::model::AppConfig;
use crate::domain::artifacts::MakeAllResult;
use crate::use_cases::context::ExecutionContext;
use crate::use_cases::request::{ArtifactsModeRequest, ArtifactsRequest, MakeAllRequest};
use crate::use_cases::result::{stamp_dispatch, UseCaseError, UseCaseFailure, UseCaseResult};
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
    for source_set in SourceSetInventory::new(config).ordered_source_sets() {
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
        };
        match super::execute(context, config, &set_request) {
            Ok(made) => result.sets.push(made),
            Err(failure) => {
                result.sets.extend(failure.payload);
                return Err(fail(failure.error, result, started));
            }
        }
    }
    result.ok = true;
    result.duration_ms = started.elapsed().as_millis() as u64;
    Ok(result)
}

/// Отказ: ответ несёт собранное до него и отказавший набор.
fn fail(
    error: UseCaseError,
    mut result: MakeAllResult,
    started: Instant,
) -> UseCaseFailure<MakeAllResult> {
    result.duration_ms = started.elapsed().as_millis() as u64;
    result.message = Some(error.to_string());
    UseCaseFailure::with_payload(error, result)
}
