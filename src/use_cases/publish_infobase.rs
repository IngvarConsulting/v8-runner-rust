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
use crate::domain::launch::DebuggerUrl;
use crate::domain::progress::ProgressPhase;
use crate::domain::publish::{
    PublishAction, PublishPlan, PublishResult, WebDebugPreparation, WebDebugState,
};
use crate::platform::locator::UtilityType;
use crate::platform::process::ProcessRequest;
use crate::platform::secrets::mask_preview_args;
use crate::platform::utilities::PlatformUtilities;
use crate::platform::webinst::{webinst_args, write_diagnostics};
use crate::support::error::AppError;
use crate::support::temp::platform_logs_dir;
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::managed_stand::ManagedPublication;
use crate::use_cases::progress::log_live_stage;
use crate::use_cases::result::{stamp_dispatch, UseCaseFailure, UseCaseResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishRequest {
    pub action: PublishAction,
    pub prepare_debug: Option<DebuggerUrl>,
    pub dry_run: bool,
}

/// Единственный выход сценария: `provider_dispatched` ответа ставит отметка работы команды.
pub fn execute(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &PublishRequest,
) -> UseCaseResult<PublishResult> {
    stamp_dispatch(run_publish(context, config, request), context.work())
}

fn run_publish(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &PublishRequest,
) -> UseCaseResult<PublishResult> {
    if let Some(ide_url) = request.prepare_debug.as_ref().filter(|_| {
        crate::use_cases::managed_stand::standalone_publishes_its_own_web_client(config)
    }) {
        let status = crate::use_cases::managed_stand::standalone_web_status(context, config)
            .map_err(UseCaseFailure::without_payload)?;
        let mut web = config.infobase.web.clone().unwrap_or_default();
        web.url = web.url.or(status.http.filter(|_| status.ready));
        web.debugger_url = status
            .debug_url
            .as_deref()
            .filter(|_| status.ready)
            .and_then(DebuggerUrl::parse);
        return prepare_web_debug(context, &web, None, true, ide_url, request.dry_run);
    }
    let managed = crate::use_cases::managed_stand::publication(
        context,
        config,
        request.prepare_debug.is_none().then_some(request.action),
        request.dry_run,
    )
    .map_err(UseCaseFailure::without_payload)?;
    publish_with(context, config, request, managed)
}

/// Выход сценария с уже выбранным стендом; его берут сценарии частного Apache с подделанной
/// машиной.
#[cfg(test)]
pub(crate) fn execute_with(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &PublishRequest,
    managed: Option<ManagedPublication<'_>>,
) -> UseCaseResult<PublishResult> {
    stamp_dispatch(
        publish_with(context, config, request, managed),
        context.work(),
    )
}

/// Публикация с уже выбранным стендом: у базы управляемого кластера `managed` несёт частный
/// Apache, его исполнителя и `webinst` выпуска кластера.
fn publish_with(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &PublishRequest,
    managed: Option<ManagedPublication<'_>>,
) -> UseCaseResult<PublishResult> {
    let started = Instant::now();
    debug!(
        command = context.command().as_str(),
        action = request.action.as_str(),
        dry_run = request.dry_run,
        managed = managed.is_some(),
        "executing publish use case"
    );
    let web = managed
        .as_ref()
        .map(|publication| &publication.target)
        .or(config.infobase.web.as_ref())
        .ok_or_else(|| {
            UseCaseFailure::without_payload(AppError::Validation(
                "infobase.web is not declared: name the web server, wsdir and dir to publish"
                    .to_owned(),
            ))
        })?;
    if let Some(ide_url) = &request.prepare_debug {
        return prepare_web_debug(
            context,
            web,
            managed.as_ref(),
            false,
            ide_url,
            request.dry_run,
        );
    }
    if managed.is_none() && web.debugger_url.is_some() && request.action == PublishAction::Publish {
        return Err(UseCaseFailure::without_payload(AppError::Validation(
            "publish would replace the external default.vrd and remove HTTP debugging; ask the external publication owner to use webinst with a -descriptor template that preserves the debug settings".to_owned(),
        )));
    }
    let server = web.server.ok_or_else(|| {
        UseCaseFailure::without_payload(AppError::Validation(
            "infobase.web.server is not declared: iis, apache2, apache22 or apache24".to_owned(),
        ))
    })?;
    // Валидация конфига уже потребовала оба поля вместе с сервером; здесь только чтение.
    let wsdir = web.wsdir.clone().unwrap_or_default();
    let dir = web.dir.clone().unwrap_or_default();

    let mut utilities = PlatformUtilities::from_config(config);
    let (receipt, program) = if let Some(publication) = &managed {
        let receipt = config
            .provider_plan(Operation::Publish)
            .receipt_for(Provider::Webinst, Vec::new());
        let program = publication.webinst.clone().ok_or_else(|| {
            UseCaseFailure::without_payload(AppError::Runtime(
                "missing managed webinst program".to_owned(),
            ))
        })?;
        (receipt, program)
    } else {
        let selected = crate::use_cases::provider_selection::select(
            config,
            &mut utilities,
            Operation::Publish,
        )
        .map_err(|(error, _receipt)| UseCaseFailure::without_payload(error))?;
        if selected.provider != Provider::Webinst {
            return Err(UseCaseFailure::without_payload(
                crate::use_cases::unimplemented_provider(Operation::Publish, selected.provider),
            ));
        }
        let Some(location) = selected.location else {
            return Err(UseCaseFailure::without_payload(
                crate::use_cases::unimplemented_provider(Operation::Publish, selected.provider),
            ));
        };
        (selected.receipt, location.path)
    };
    let args = webinst_args(
        request.action,
        server,
        web,
        &wsdir,
        &dir,
        managed
            .as_ref()
            .map_or(config.infobase.connection.as_str(), |target| {
                target.connection.as_str()
            }),
    );
    // На Linux управляемая публикация поднимает права только у `webinst`.
    let (program, args) = match &managed {
        Some(publication) => publication.command(program, args),
        None => (program, args),
    };
    let secrets: Vec<&str> = config.infobase.password.as_deref().into_iter().collect();
    let result = |plan: Option<PublishPlan>| PublishResult {
        provider: Some(receipt.clone()),
        ok: true,
        provider_dispatched: false,
        action: request.action,
        server: server.as_str().to_owned(),
        wsdir: wsdir.clone(),
        dir: dir.clone(),
        url: web.url.clone(),
        debug: None,
        plan,
        platform_log_path: None,
        duration_ms: started.elapsed().as_millis() as u64,
        message: None,
    };

    if request.dry_run {
        log_live_stage(
            ProgressPhase::Preview,
            None,
            "publish: preview",
            "[webinst] preview only, web server not touched",
        );
        let mut preview = result(Some(PublishPlan {
            program: program.clone(),
            // `-connstr` несёт строку соединения целиком, а в ней бывает `Pwd=`.
            args: mask_preview_args(&args, &secrets),
        }));
        preview.message = Some(format!(
            "would {} '{}' on {} via {}; web server not touched",
            request.action.as_str(),
            wsdir,
            server.as_str(),
            program.display()
        ));
        return Ok(preview);
    }

    let logs_dir = match &managed {
        Some(publication) => Ok(publication.log_root.clone()),
        None => platform_logs_dir(&config.work_path),
    }
    .map_err(|error| {
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
        ProgressPhase::Publish,
        None,
        "publish: webinst",
        &format!(
            "[webinst] {} '{}' on {}",
            request.action.as_str(),
            wsdir,
            server.as_str()
        ),
    );
    let snapshot = managed
        .as_ref()
        .map(|publication| publication.snapshot())
        .transpose()
        .map_err(UseCaseFailure::without_payload)?;
    let process = ProcessRequest {
        program,
        args,
        workdir: None,
        stdout_log_path: None,
        stderr_log_path: None,
        startup_probe: None,
        windows_verbatim_args: Vec::new(),
    };
    let runner = match &managed {
        Some(publication) => publication.runner(),
        None => utilities.runner_for(UtilityType::Webinst),
    };
    let finished = runner
        .run_with_policy(
            &process,
            &context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
        )
        .and_then(|finished| {
            write_diagnostics(&log_path, &finished, &secrets)?;
            Ok(finished)
        })
        .map_err(|error| {
            // Отказ после старта `webinst` отвечает формой `publish`. Журнала у оборванного
            // запуска нет — исполнитель пишет его только после выхода, — поэтому и пути к
            // нему ответ не называет.
            let error = AppError::from(error);
            let message = error.to_string();
            UseCaseFailure::after_possible_work(error, context.work(), || {
                let mut failed = result(None);
                failed.ok = false;
                failed.message = Some(message);
                failed
            })
        });
    let finished = match finished {
        Ok(finished) => finished,
        Err(mut failure) => {
            if let (Some(publication), Some(snapshot)) = (&managed, snapshot) {
                for rollback in [snapshot.restore().err(), publication.reclaim_files().err()]
                    .into_iter()
                    .flatten()
                {
                    failure.error = failure.error.followed_by(rollback.to_string());
                }
            }
            return Err(failure);
        }
    };

    let mut done = result(None);
    done.platform_log_path = Some(log_path);
    match finished.outcome() {
        Ok(()) => {
            if let Some(publication) = &managed {
                // Сначала файлы `webinst` возвращаются пользователю раннера, затем Apache
                // перечитывает конфиг.
                let debugger = web
                    .debugger_url
                    .as_ref()
                    .filter(|_| request.action == PublishAction::Publish);
                let activated = snapshot
                    .ok_or_else(|| {
                        AppError::Runtime("missing managed publication snapshot".to_owned()).into()
                    })
                    .and_then(|snapshot| publication.activate_publish(snapshot, debugger));
                if let Err(error) = activated {
                    done.ok = false;
                    done.message = Some(error.to_string());
                    return Err(UseCaseFailure::with_payload(error, done));
                }
            }
            done.message = Some(format!(
                "{} '{}' on {} completed",
                request.action.as_str(),
                wsdir,
                server.as_str()
            ));
            Ok(done)
        }
        Err(code) => {
            done.ok = false;
            let message = format!(
                "webinst exited with status {code} while trying to {} '{}' on {}",
                request.action.as_str(),
                wsdir,
                server.as_str()
            );
            let mut error = managed
                .as_ref()
                .and_then(|publication| publication.elevation_refusal(&finished))
                .unwrap_or_else(|| {
                    crate::use_cases::result::UseCaseError::from(AppError::Platform(message))
                });
            done.message = Some(error.to_string());
            if let (Some(publication), Some(snapshot)) = (&managed, snapshot) {
                for rollback in [snapshot.restore().err(), publication.reclaim_files().err()]
                    .into_iter()
                    .flatten()
                {
                    error = error.followed_by(rollback.to_string());
                }
            }
            Err(UseCaseFailure::with_payload(error, done))
        }
    }
}

fn prepare_web_debug(
    context: &ExecutionContext,
    web: &crate::config::model::InfobaseWebConfig,
    managed: Option<&ManagedPublication<'_>>,
    standalone: bool,
    ide_url: &DebuggerUrl,
    dry_run: bool,
) -> UseCaseResult<PublishResult> {
    let started = Instant::now();
    let result = |state, publication_changed, message: String| PublishResult {
        provider: None,
        ok: state != WebDebugState::RouteUnavailable,
        provider_dispatched: false,
        action: PublishAction::Publish,
        server: web
            .server
            .map_or(if standalone { "ibsrv" } else { "external" }, |server| {
                server.as_str()
            })
            .to_owned(),
        wsdir: web.wsdir.clone().unwrap_or_default(),
        dir: web.dir.clone().unwrap_or_default(),
        url: web
            .url
            .as_deref()
            .map(crate::platform::secrets::mask_url_userinfo),
        debug: Some(WebDebugPreparation {
            state,
            ide_url: ide_url.clone(),
            debugger_url: web.debugger_url.clone(),
            publication_changed,
        }),
        plan: None,
        platform_log_path: None,
        duration_ms: started.elapsed().as_millis() as u64,
        message: Some(message),
    };
    let unavailable = |message: String| {
        let error = AppError::environment(message.clone());
        UseCaseFailure::with_payload(
            error,
            result(WebDebugState::RouteUnavailable, false, message),
        )
    };
    let debugger = web.debugger_url.as_ref().ok_or_else(|| unavailable(if standalone {
        "start the standalone server with `standalone start --debug` or enable debugging with `standalone restart --debug`".to_owned()
    } else {
        "declare infobase.web.debugger-url as the direct HTTP debugger address reachable from the publication host".to_owned()
    }))?;
    if web
        .url
        .as_deref()
        .is_none_or(|url| crate::platform::browser::with_http_debug(url, debugger).is_err())
    {
        return Err(unavailable(
            "declare infobase.web.url for the browser".to_owned(),
        ));
    }
    if dry_run {
        return Ok(result(
            WebDebugState::Planned,
            false,
            "preview: would check the publication's HTTP debug route".to_owned(),
        ));
    }
    if let Some(cancel) = crate::use_cases::interruption::SafePointCancel::noticed(
        context,
        crate::use_cases::interruption::SafePoint::Command,
    ) {
        return Err(UseCaseFailure::without_payload(cancel.into_error()));
    }
    if managed.is_none() && !standalone {
        let descriptor = web.dir.as_ref().map(|dir| dir.join("default.vrd")).ok_or_else(|| unavailable(
            "run publication preparation on the external web server host and declare infobase.web.dir for its active default.vrd".to_owned(),
        ))?;
        let contents = std::fs::read(&descriptor).map_err(|error| unavailable(format!(
            "cannot read '{}': {error}; point infobase.web.dir at the active publication on the web server host", descriptor.display(),
        )))?;
        let edit = crate::platform::webinst::publication_debug_edit(&contents, debugger).map_err(
            |error| {
                unavailable(format!(
                    "cannot read publication debug settings in '{}': {error}",
                    descriptor.display()
                ))
            },
        )?;
        if edit.is_some() {
            return Err(unavailable(format!(
                "ask the external publication owner to enable HTTP debugging in '{}' with debugger URL {debugger} and reload the publication",
                descriptor.display(),
            )));
        }
    }
    let route_answers = crate::platform::dbgs::probe_rdbg(&debugger.to_string());
    if let Some(cancel) = crate::use_cases::interruption::SafePointCancel::noticed(
        context,
        crate::use_cases::interruption::SafePoint::Command,
    ) {
        return Err(UseCaseFailure::without_payload(cancel.into_error()));
    }
    if !route_answers {
        return Err(unavailable(format!(
            "rdbg did not answer at {debugger} from this host; start the debugger server and ask the publication owner to make infobase.web.debugger-url reachable from the web server",
        )));
    }
    if let Some(publication) = managed {
        let changed = publication.prepare_debug(debugger).map_err(|error| {
            let message = error.to_string();
            UseCaseFailure::with_payload(
                error,
                result(WebDebugState::RouteUnavailable, false, message),
            )
        })?;
        return Ok(result(
            WebDebugState::Ready,
            changed,
            "publication HTTP debugging is ready; connect the IDE debugger before opening the browser".to_owned(),
        ));
    }
    if standalone {
        Ok(result(WebDebugState::Ready, false, "standalone HTTP debugging is ready; connect the IDE debugger before opening the browser".to_owned()))
    } else {
        Ok(result(WebDebugState::ExternalRouteVerified, false, "the descriptor and debugger route are verified from this host; the external publication owner must activate descriptor changes".to_owned()))
    }
}
