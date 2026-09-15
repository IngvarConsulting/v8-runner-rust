//! `publish` и `publish --delete`: публикация базы на веб-сервере через `webinst`.
//!
//! Параметры берутся из `infobase.web`, а не из флагов: публикация должна
//! воспроизводиться из файла. Развилки у операции нет — исполнитель один, `webinst`,
//! и `providers.publish` валидация отклоняет. Публикация замещает `default.vrd`
//! целиком, поэтому у команды есть превью, а удаление — отдельный явный ключ.

use std::time::Instant;

use tracing::debug;

use crate::config::model::AppConfig;
use crate::domain::capability::{Operation, Provider};
use crate::domain::publish::{PublishAction, PublishPlan, PublishResult};
use crate::platform::locator::UtilityType;
use crate::platform::process::ProcessRequest;
use crate::platform::utilities::PlatformUtilities;
use crate::platform::webinst::webinst_args;
use crate::support::error::AppError;
use crate::support::temp::platform_logs_dir;
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::progress::log_live_stage;
use crate::use_cases::result::{UseCaseFailure, UseCaseResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishRequest {
    pub action: PublishAction,
    pub dry_run: bool,
}

#[allow(clippy::result_large_err)] // Failure payload preserves the typed result.
pub fn execute(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &PublishRequest,
) -> UseCaseResult<PublishResult> {
    let started = Instant::now();
    debug!(
        command = context.command().as_str(),
        action = request.action.as_str(),
        dry_run = request.dry_run,
        "executing publish use case"
    );

    let web = config.infobase.web.as_ref().ok_or_else(|| {
        UseCaseFailure::without_payload(AppError::Validation(
            "infobase.web is not declared: name the web server, wsdir and dir to publish"
                .to_owned(),
        ))
    })?;
    let server = web.server.ok_or_else(|| {
        UseCaseFailure::without_payload(AppError::Validation(
            "infobase.web.server is not declared: iis, apache2, apache22 or apache24".to_owned(),
        ))
    })?;
    // Валидация конфига уже потребовала оба поля вместе с сервером; здесь только чтение.
    let wsdir = web.wsdir.clone().unwrap_or_default();
    let dir = web.dir.clone().unwrap_or_default();

    let provider = config.selected_provider(Operation::Publish);
    if provider != Provider::Webinst {
        return Err(UseCaseFailure::without_payload(
            crate::use_cases::unimplemented_provider(Operation::Publish, provider),
        ));
    }

    let mut utilities = PlatformUtilities::from_config(config);
    let location = utilities
        .locate(UtilityType::Webinst)
        .map_err(|error| UseCaseFailure::without_payload(AppError::from(error)))?;

    let args = webinst_args(
        request.action,
        server,
        web,
        &wsdir,
        &dir,
        &config.infobase.connection,
    );
    let result = |provider_dispatched: bool, plan: Option<PublishPlan>| PublishResult {
        ok: true,
        provider_dispatched,
        action: request.action,
        server: server.as_str().to_owned(),
        wsdir: wsdir.clone(),
        dir: dir.clone(),
        url: web.url.clone(),
        plan,
        platform_log_path: None,
        duration_ms: started.elapsed().as_millis() as u64,
        message: None,
    };

    if request.dry_run {
        log_live_stage(
            "publish: preview",
            "[webinst] preview only, web server not touched",
        );
        let mut preview = result(
            false,
            Some(PublishPlan {
                program: location.path.clone(),
                args: args.clone(),
            }),
        );
        preview.message = Some(format!(
            "would {} '{}' on {} via {}; web server not touched",
            request.action.as_str(),
            wsdir,
            server.as_str(),
            location.path.display()
        ));
        return Ok(preview);
    }

    let logs_dir = platform_logs_dir(&config.work_path).map_err(|error| {
        UseCaseFailure::without_payload(AppError::Runtime(format!(
            "failed to create platform log directory under '{}': {error}",
            config.work_path.display()
        )))
    })?;
    let log_path = logs_dir.join(format!(
        "publish_{}_{}.log",
        request.action.as_str(),
        std::process::id()
    ));
    log_live_stage(
        "publish: webinst",
        &format!(
            "[webinst] {} '{}' on {}",
            request.action.as_str(),
            wsdir,
            server.as_str()
        ),
    );
    let process = ProcessRequest {
        program: location.path.clone(),
        args,
        workdir: None,
        stdout_log_path: Some(log_path.clone()),
        stderr_log_path: Some(log_path.clone()),
        startup_probe: None,
    };
    let runner = utilities.runner_for(UtilityType::Webinst);
    let outcome = runner
        .run_with_policy(
            &process,
            &context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
        )
        .map_err(|error| UseCaseFailure::without_payload(AppError::from(error)))?;

    let mut done = result(true, None);
    done.platform_log_path = Some(log_path);
    if outcome.exit_code == 0 {
        done.message = Some(format!(
            "{} '{}' on {} completed",
            request.action.as_str(),
            wsdir,
            server.as_str()
        ));
        Ok(done)
    } else {
        done.ok = false;
        let message = format!(
            "webinst exited with status {} while trying to {} '{}' on {}",
            outcome.exit_code,
            request.action.as_str(),
            wsdir,
            server.as_str()
        );
        done.message = Some(message.clone());
        Err(UseCaseFailure::with_payload(
            AppError::Platform(message),
            done,
        ))
    }
}
