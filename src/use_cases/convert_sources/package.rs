//! Направления `convert` с пакетом: наборы в `.cf` и `.cfe` и файл пакета в XML платформы.
//!
//! Исполнителя выбирает строка `convert` матрицы (`provider_selection`), и квитанция
//! называет его в ответе. `ibcmd` работает во временной базе раннера под `workPath`
//! ([`ThrowawayInfobase`]) — тем же владельцем, что у `make`: сборка — `config import --out`,
//! разбор — `config export --file`. База проекта не выбирается и не открывается. Исходники
//! формата EDT сперва переводит в XML `1cedtcli` шагом сборки `push` в каталог временной базы.
//!
//! Пакет публикуется заменой файла, как у `make`; каталог XML — заменой каталога со
//! сторожем незафиксированной работы, как у перевода между EDT и XML.

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::config::model::{AppConfig, SourceSetConfig};
use crate::domain::capability::Operation;
use crate::domain::convert::{ConvertDirection, ConvertOutput, ConvertResult};
use crate::platform::locator::UtilityType;
use crate::platform::process::ProcessRunner;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::support::path::{
    is_filesystem_root, nearest_existing_canonical_path, stable_path_identity,
};
use crate::use_cases::context::{CommandName, ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::destruction_guard::{
    discard_note, losses_in, preview_note, Destruction, DestructionConsent, WaysOut,
};
use crate::use_cases::progress::log_live_stage;
use crate::use_cases::provider_selection::{self, SelectedProvider};
use crate::use_cases::request::{ConvertRequest, ConvertScopeRequest};
use crate::use_cases::result::UseCaseResult;
use crate::use_cases::source_inventory::{package_in_directory, paths_overlap, SourceSetInventory};
use crate::use_cases::staged_publication::StagedPublication;
use crate::use_cases::throwaway_infobase::{Builder, Package, ThrowawayInfobase};

use super::{
    convert_workspace_path, deferred_interruption_warning, ensure_platform_success,
    explicit_output_root, merge_messages, require_source_sets, result_snapshot, scope_from_request,
    source_set_from_request, validate_convert_target, validate_designer_layout,
    validate_selected_source, ConvertExecutionFailure, CONVERT_BACKUP_PREFIX,
};

/// Префикс промежуточного файла и каталога рядом с целью.
const STAGE_PREFIX: &str = ".convert-stage";

/// Что переводится и куда.
#[derive(Debug, Clone)]
enum Input {
    /// Набор проекта в пакет: основная конфигурация или расширение с его именем в базе.
    SourceSet {
        name: String,
        extension: Option<String>,
    },
    /// Файл пакета в каталог XML.
    PackageFile,
}

#[derive(Debug, Clone)]
struct Item {
    input: Input,
    source_path: PathBuf,
    target_path: PathBuf,
    target_identity: String,
}

impl Item {
    fn source_set(&self) -> Option<&str> {
        match &self.input {
            Input::SourceSet { name, .. } => Some(name),
            Input::PackageFile => None,
        }
    }

    fn output(&self) -> ConvertOutput {
        ConvertOutput {
            source_set: self.source_set().map(ToOwned::to_owned),
            source_path: self.source_path.clone(),
            target_path: self.target_path.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedPackageRequest {
    items: Vec<Item>,
    /// Согласие на замену каталога XML; у пакета-файла сторожа нет.
    consent: DestructionConsent,
}

/// Разрешает входы и цели направления с пакетом без обращения к платформе.
pub(super) fn resolve(
    config: &AppConfig,
    request: &ConvertRequest,
    direction: ConvertDirection,
) -> Result<ResolvedPackageRequest, AppError> {
    let explicit_output_root = explicit_output_root(request)?;
    let items = match (&request.scope, direction) {
        (ConvertScopeRequest::Package { path }, ConvertDirection::PackageToDesigner) => {
            vec![resolve_package_file(
                config,
                path,
                explicit_output_root.as_deref(),
            )?]
        }
        (ConvertScopeRequest::All | ConvertScopeRequest::SourceSet { .. }, _)
            if direction != ConvertDirection::PackageToDesigner =>
        {
            resolve_source_sets(config, request, direction, explicit_output_root.as_deref())?
        }
        _ => {
            return Err(AppError::Validation(format!(
                "convert direction {direction:?} does not take this input"
            )))
        }
    };
    Ok(ResolvedPackageRequest {
        items,
        // Вывод по умолчанию лежит под `workPath` — месте раннера; каталог, названный
        // `--output`, принадлежит человеку.
        consent: if explicit_output_root.is_none() {
            DestructionConsent::RunnerOwned
        } else if request.discard_uncommitted {
            DestructionConsent::Granted
        } else {
            DestructionConsent::AskFirst(WaysOut::SameCallWithForce)
        },
    })
}

/// Наборы в пакеты: без набора — пакеты конфигурации проекта в порядке обхода, с набором —
/// его пакет; у набора внешних файлов пакета конфигурации нет, и это отказ.
fn resolve_source_sets(
    config: &AppConfig,
    request: &ConvertRequest,
    direction: ConvertDirection,
    explicit_output_root: Option<&Path>,
) -> Result<Vec<Item>, AppError> {
    require_source_sets(config)?;
    let inventory = SourceSetInventory::new(config);
    let packages: Vec<(&SourceSetConfig, Option<&str>)> = match &request.scope {
        ConvertScopeRequest::SourceSet { name } => {
            vec![inventory.configuration_package(name, CommandName::Convert)?]
        }
        _ => inventory.configuration_packages(),
    };
    if packages.is_empty() {
        return Err(AppError::Validation(
            "convert --to package takes configuration and extension source-sets, and the project declares none".to_owned(),
        ));
    }
    let root = explicit_output_root
        .map(Path::to_path_buf)
        .unwrap_or_else(|| {
            config
                .work_path
                .join("convert")
                .join("out")
                .join("packages")
        });
    let mut items: Vec<Item> = Vec::with_capacity(packages.len());
    for (source_set, extension) in packages {
        let source_path = source_set.root_in(&config.base_path);
        validate_selected_source(source_set, direction, &source_path)?;
        let target_path = package_in_directory(&root, source_set);
        let subject = format!("source-set '{}'", source_set.name);
        let (canonical, target_identity) = checked_target(
            config,
            &target_path,
            &subject,
            explicit_output_root.is_some(),
        )?;
        if let Some(other) = items.iter().find(|item| {
            paths_overlap(
                &nearest_existing_canonical_path(&item.target_path)
                    .unwrap_or_else(|_| item.target_path.clone()),
                &canonical,
            )
        }) {
            return Err(AppError::Validation(format!(
                "convert output targets overlap: {} -> {}, {subject} -> {}",
                other
                    .source_set()
                    .map(|name| format!("source-set '{name}'"))
                    .unwrap_or_default(),
                other.target_path.display(),
                target_path.display()
            )));
        }
        items.push(Item {
            input: Input::SourceSet {
                name: source_set.name.clone(),
                extension: extension.map(ToOwned::to_owned),
            },
            source_path,
            target_path,
            target_identity,
        });
    }
    Ok(items)
}

/// Файл пакета в XML: файл есть и это файл; каталог XML — `--output` или
/// `workPath/convert/out/<имя файла>/designer`, и сам файл в него не попадает.
fn resolve_package_file(
    config: &AppConfig,
    path: &str,
    explicit_output_root: Option<&Path>,
) -> Result<Item, AppError> {
    let trimmed = path.trim();
    let source_path = std::path::absolute(trimmed)
        .map(|path| crate::support::path::lexically_normal_absolute(&path))
        .map_err(|error| {
            AppError::Runtime(format!(
                "failed to resolve package file '{trimmed}' against the current directory: {error}"
            ))
        })?;
    if !source_path.is_file() {
        return Err(AppError::Validation(format!(
            "convert package file does not exist or is not a file: {}",
            source_path.display()
        )));
    }
    let name = source_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| trimmed.to_owned());
    let target_path = match explicit_output_root {
        Some(root) => root.to_path_buf(),
        None => {
            let stem = source_path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| name.clone());
            config
                .work_path
                .join("convert")
                .join("out")
                .join(stem)
                .join("designer")
        }
    };
    let subject = format!("package '{name}'");
    let (canonical, target_identity) = checked_target(
        config,
        &target_path,
        &subject,
        explicit_output_root.is_some(),
    )?;
    let canonical_source =
        nearest_existing_canonical_path(&source_path).unwrap_or_else(|_| source_path.clone());
    if paths_overlap(&canonical, &canonical_source) {
        return Err(AppError::Validation(format!(
            "convert output for {subject} would replace the package file itself: target={}",
            target_path.display()
        )));
    }
    Ok(Item {
        input: Input::PackageFile,
        source_path,
        target_path,
        target_identity,
    })
}

/// Цель сверяется с наборами проекта, `basePath` и `workPath` так же, как у перевода между
/// EDT и XML; ответ — её сравнимый путь и тождество для промежуточных следов.
fn checked_target(
    config: &AppConfig,
    target_path: &Path,
    subject: &str,
    is_explicit_output: bool,
) -> Result<(PathBuf, String), AppError> {
    validate_convert_target(config, target_path, subject, is_explicit_output)?;
    let canonical = nearest_existing_canonical_path(target_path).map_err(|error| {
        AppError::Runtime(format!(
            "failed to canonicalize convert output '{}': {error}",
            target_path.display()
        ))
    })?;
    if is_filesystem_root(&canonical) {
        return Err(AppError::Validation(
            "convert output target must not equal filesystem root".to_owned(),
        ));
    }
    let identity = stable_path_identity(&canonical);
    Ok((canonical, identity))
}

/// Направление с пакетом: разрешение, выбор исполнителя по строке `convert`, превью или
/// работа во временной базе раннера. Квитанция едет и в ответе, и в отказе после выбора.
pub(super) fn run(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ConvertRequest,
    direction: ConvertDirection,
    started: Instant,
) -> UseCaseResult<ConvertResult> {
    let workspace_path = needs_edt(direction).then(|| convert_workspace_path(config));
    let snapshot = |ok: bool, outputs: Vec<ConvertOutput>, message: Option<String>| {
        result_snapshot(
            ok,
            direction,
            scope_from_request(request),
            source_set_from_request(request),
            workspace_path.clone(),
            outputs,
            started,
            message,
        )
    };
    let fail = |error: AppError, outputs: Vec<ConvertOutput>| {
        let message = error.to_string();
        ConvertExecutionFailure::with_payload(error, snapshot(false, outputs, Some(message)))
    };

    let resolved = resolve(config, request, direction).map_err(|error| fail(error, Vec::new()))?;
    let mut utilities = PlatformUtilities::from_config(config);
    let selected = match provider_selection::select(config, &mut utilities, Operation::Convert) {
        Ok(selected) => selected,
        Err((error, receipt)) => {
            let mut failure = fail(error, Vec::new());
            if let Some(payload) = failure.payload.as_mut() {
                payload.provider = Some(receipt);
            }
            return Err(failure);
        }
    };
    let receipt = selected.receipt.clone();
    let outcome = run_selected(
        context,
        config,
        request,
        direction,
        &resolved,
        selected,
        &mut utilities,
        &snapshot,
    );
    provider_selection::attach(outcome, &receipt)
}

fn needs_edt(direction: ConvertDirection) -> bool {
    direction == ConvertDirection::EdtToPackage
}

#[allow(clippy::too_many_arguments)]
fn run_selected(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ConvertRequest,
    direction: ConvertDirection,
    resolved: &ResolvedPackageRequest,
    selected: SelectedProvider,
    utilities: &mut PlatformUtilities,
    snapshot: &dyn Fn(bool, Vec<ConvertOutput>, Option<String>) -> ConvertResult,
) -> UseCaseResult<ConvertResult> {
    let fail = |error: AppError, outputs: Vec<ConvertOutput>| {
        let message = error.to_string();
        ConvertExecutionFailure::with_payload(error, snapshot(false, outputs, Some(message)))
    };
    let Some(binary) = selected.location.map(|location| location.path) else {
        return Err(fail(
            crate::use_cases::unimplemented_provider(Operation::Convert, selected.provider),
            Vec::new(),
        ));
    };
    // Исходники EDT сперва переводит `1cedtcli`: без него превью отказывает так же, как
    // отказал бы прогон.
    let edt_cli = if needs_edt(direction) {
        Some(
            utilities
                .locate(UtilityType::EdtCli)
                .map_err(|error| fail(AppError::from(error), Vec::new()))?
                .path,
        )
    } else {
        None
    };

    if request.dry_run {
        let mut message = format!(
            "previewed conversion via {} {}; {} not dispatched",
            selected.provider,
            binary.display(),
            selected.provider
        );
        for item in &resolved.items {
            if matches!(item.input, Input::PackageFile)
                && !matches!(resolved.consent, DestructionConsent::RunnerOwned)
            {
                let losses = losses_in(&item.target_path, &[]);
                if let Some(note) = preview_note(
                    context,
                    &item.target_path,
                    &resolved.consent,
                    &losses,
                    Destruction::Replace,
                ) {
                    message.push_str("; ");
                    message.push_str(&note);
                }
            }
        }
        let outputs = resolved.items.iter().map(Item::output).collect();
        return Ok(snapshot(true, outputs, Some(message)));
    }

    let runner = utilities.runner_for(UtilityType::Ibcmd);
    let edt = edt_cli.map(|path| {
        crate::platform::edt::EdtDsl::new(
            path,
            convert_workspace_path(config),
            utilities.runner_for(UtilityType::EdtCli),
            context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
        )
        .with_timeout(context.edt_timeout())
    });
    let base = ThrowawayInfobase::create(
        context,
        &config.work_path,
        Builder {
            provider: selected.provider,
            binary,
        },
        runner,
    )
    .map_err(|error| fail(error, Vec::new()))?;

    let mut outputs = Vec::new();
    let mut messages = Vec::new();
    let mut base = base;
    for item in &resolved.items {
        let converted = convert_item(
            context,
            config,
            &mut base,
            runner,
            edt.as_ref(),
            resolved,
            item,
        );
        match converted {
            Ok(notes) => {
                messages.extend(notes);
                outputs.push(item.output());
            }
            Err(error) => {
                // База убирается и после отказа; неудачная уборка едет с отказом.
                let warnings = base.close();
                let error = if warnings.is_empty() {
                    error
                } else {
                    error.with_context(warnings.join("; "))
                };
                return Err(fail(error, outputs));
            }
        }
    }
    messages.extend(base.close());
    Ok(snapshot(true, outputs, merge_messages(messages)))
}

/// Один вход: сборка пакета или разбор пакета во временной базе и публикация. Ответ —
/// предупреждения публикации.
fn convert_item(
    context: &ExecutionContext,
    config: &AppConfig,
    base: &mut ThrowawayInfobase,
    runner: &dyn ProcessRunner,
    edt: Option<&crate::platform::edt::EdtDsl<'_>>,
    resolved: &ResolvedPackageRequest,
    item: &Item,
) -> Result<Vec<String>, AppError> {
    match &item.input {
        Input::SourceSet { name, extension } => {
            let source_dir = match edt {
                Some(edt) => edt_sources_in_xml(context, config, edt, base, name)?,
                None => item.source_path.clone(),
            };
            build_package(
                context,
                base,
                runner,
                item,
                name,
                extension.as_deref(),
                &source_dir,
            )
        }
        Input::PackageFile => export_package(context, base, runner, resolved, item),
    }
}

/// Исходники EDT в XML: `1cedtcli` шагом сборки `push` в каталог временной базы, как у
/// `make`.
fn edt_sources_in_xml(
    context: &ExecutionContext,
    config: &AppConfig,
    edt: &crate::platform::edt::EdtDsl<'_>,
    base: &ThrowawayInfobase,
    name: &str,
) -> Result<PathBuf, AppError> {
    let inventory = SourceSetInventory::new(config);
    let source_set = inventory.named(name)?;
    let edt_context = inventory
        .edt_context(name)
        .ok_or_else(|| AppError::Runtime(format!("missing EDT context for source-set '{name}'")))?;
    let target = base.xml_dir(name);
    log_live_stage("convert: edt export", "[EDT] converting the sources to XML");
    crate::use_cases::build_project::execute_edt_export_step(
        context,
        config,
        edt,
        source_set,
        edt_context,
        &target,
        "convert",
    )?;
    Ok(target)
}

fn build_package(
    context: &ExecutionContext,
    base: &mut ThrowawayInfobase,
    runner: &dyn ProcessRunner,
    item: &Item,
    name: &str,
    extension: Option<&str>,
    source_dir: &Path,
) -> Result<Vec<String>, AppError> {
    let file_extension = item
        .target_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("cf");
    let publication = StagedPublication::prepare_file(
        &item.target_path,
        &item.target_identity,
        STAGE_PREFIX,
        file_extension,
    )?;
    let staging_file = publication.staging_path().to_path_buf();
    let package = match extension {
        Some(extension) => Package::Extension(extension),
        None => Package::Configuration,
    };
    let built = base
        .build_package(
            context,
            runner,
            name,
            source_dir,
            package,
            &staging_file,
            None,
        )
        .and_then(|result| {
            ensure_platform_success(name, "designer-to-package", &result)?;
            if staging_file.is_file() {
                Ok(())
            } else {
                Err(AppError::Platform(format!(
                    "{} did not produce package file '{}'",
                    base.provider(),
                    staging_file.display()
                )))
            }
        });
    if let Err(error) = built {
        return Err(publication.cleanup_failure(error));
    }
    // Неудачная замена оставляет промежуточный файл: в нём может быть единственная копия.
    let published = publication.publish_file(context, "failed to publish convert package")?;
    let mut notes = Vec::new();
    notes.extend(published.cleanup_warning);
    notes.extend(deferred_interruption_warning(
        published.deferred_interruption,
    ));
    Ok(notes)
}

fn export_package(
    context: &ExecutionContext,
    base: &mut ThrowawayInfobase,
    runner: &dyn ProcessRunner,
    resolved: &ResolvedPackageRequest,
    item: &Item,
) -> Result<Vec<String>, AppError> {
    let label = item
        .source_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let publication =
        StagedPublication::prepare_dir(&item.target_path, &item.target_identity, STAGE_PREFIX)?;
    let staging_dir = publication.staging_path().to_path_buf();
    let exported = base
        .export_package(context, runner, &item.source_path, &staging_dir)
        .and_then(|result| {
            ensure_platform_success(&label, "package-to-designer", &result)?;
            validate_designer_layout(&staging_dir, "Designer convert output")
        });
    if let Err(error) = exported {
        return Err(publication.cleanup_failure(error));
    }
    let published = publication.publish_dir(
        context,
        CONVERT_BACKUP_PREFIX,
        "failed to publish convert output",
        &resolved.consent,
        &[],
    )?;
    let mut notes = Vec::new();
    notes.extend(discard_note(&item.target_path, &published.discarded));
    notes.extend(published.cleanup_warning);
    notes.extend(deferred_interruption_warning(
        published.deferred_interruption,
    ));
    Ok(notes)
}
