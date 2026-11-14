use serde::Serialize;

use crate::command_envelope::{Envelope, EnvelopeError, ErrorCode, ErrorKind};
use crate::output::presenter::Presenter;
use crate::support::error::CapabilityReason;
use crate::use_cases::context::CommandName;
use crate::use_cases::result::{EnvironmentReason, UseCaseError, UseCaseErrorKind};

pub fn print_command_error(
    presenter: &Presenter,
    command: &str,
    error: &UseCaseError,
    text_message: &str,
) {
    if presenter.is_json() {
        presenter.print_envelope(&pre_dispatch_error_envelope(command, error));
    } else {
        presenter.print_error(text_message);
    }
}

pub fn print_command_use_case_error(
    presenter: &Presenter,
    command: CommandName,
    error: &UseCaseError,
) {
    print_command_error(presenter, command.as_str(), error, &error.to_string());
}

/// `data` отказа, случившегося до того, как команда начала работу.
///
/// Предмета у такого ответа нет — платформа не запускалась, план не строился, — поэтому
/// форма несёт только текст отказа. Машинная часть причины живёт в `error` конверта.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RefusalData {
    pub message: String,
}

pub fn pre_dispatch_error_envelope(command: &str, error: &UseCaseError) -> Envelope<RefusalData> {
    let data = RefusalData {
        message: error.message().to_owned(),
    };
    failure_envelope(command, 0, data, error)
}

pub fn failure_envelope<T: Serialize>(
    command: impl Into<String>,
    duration_ms: u64,
    data: T,
    error: &UseCaseError,
) -> Envelope<T> {
    with_cli_error(Envelope::err(command, duration_ms, data), error)
}

pub fn with_cli_error<T: Serialize>(envelope: Envelope<T>, error: &UseCaseError) -> Envelope<T> {
    envelope.with_error(cli_envelope_error(error))
}

fn cli_envelope_error(error: &UseCaseError) -> EnvelopeError {
    let (code, kind) = cli_error_contract(error.kind());
    let mut envelope =
        EnvelopeError::new(code, kind, error.message()).with_next(error.next().cloned());
    if let Some(generations) = error.generations() {
        envelope.base_generation = Some(generations.base.clone());
        envelope.local_generation = Some(generations.local.clone());
    }
    envelope.platform_release = error.platform_release().cloned().map(Box::new);
    envelope.sessions = error.sessions().cloned().map(Box::new);
    envelope.layer = error.environment_layer();
    envelope
}

/// Род отказа определяет пару «код, род» на проводе. Причина возможности различает коды
/// внутри одного рода: подбор команд на сайте зовёт их `subject`, `target` и `soon`.
pub(crate) const fn cli_error_contract(kind: UseCaseErrorKind) -> (ErrorCode, ErrorKind) {
    match kind {
        UseCaseErrorKind::Capability(reason) => (
            match reason {
                CapabilityReason::Unavailable => ErrorCode::CapabilityUnavailable,
                CapabilityReason::Subject => ErrorCode::Subject,
                CapabilityReason::Target => ErrorCode::Target,
                CapabilityReason::Soon => ErrorCode::Soon,
            },
            ErrorKind::Capability,
        ),
        UseCaseErrorKind::Environment(reason) => (
            match reason {
                EnvironmentReason::Unavailable => ErrorCode::EnvironmentUnavailable,
                EnvironmentReason::PlatformReleaseMissing => ErrorCode::PlatformReleaseMissing,
            },
            ErrorKind::Environment,
        ),
        UseCaseErrorKind::WorkspaceBusy => (ErrorCode::WorkspaceBusy, ErrorKind::Workspace),
        UseCaseErrorKind::InfobaseBusy => (ErrorCode::InfobaseBusy, ErrorKind::Workspace),
        UseCaseErrorKind::InfobaseSessions => (ErrorCode::InfobaseSessions, ErrorKind::Workspace),
        UseCaseErrorKind::InfobaseDesignerBusy => {
            (ErrorCode::InfobaseDesignerBusy, ErrorKind::Workspace)
        }
        UseCaseErrorKind::NonFastForward => (ErrorCode::NonFastForward, ErrorKind::NonFastForward),
        UseCaseErrorKind::UntrackedBase => (ErrorCode::UntrackedBase, ErrorKind::UntrackedBase),
        UseCaseErrorKind::AgentLimit => (ErrorCode::AgentLimit, ErrorKind::Environment),
        UseCaseErrorKind::LiveAgentOff => (ErrorCode::LiveAgentOff, ErrorKind::Capability),
        UseCaseErrorKind::LiveAgentJob => (ErrorCode::LiveAgentJob, ErrorKind::Environment),
        UseCaseErrorKind::SourceSetIgnoredByGit => {
            (ErrorCode::SourceSetIgnoredByGit, ErrorKind::Validation)
        }
        UseCaseErrorKind::WorkPathOnAnotherVolume => {
            (ErrorCode::WorkPathOnAnotherVolume, ErrorKind::Validation)
        }
        UseCaseErrorKind::InvalidOutput => (ErrorCode::InvalidOutput, ErrorKind::InvalidOutput),
        UseCaseErrorKind::Cancelled(_) => (ErrorCode::Cancelled, ErrorKind::Interruption),
        UseCaseErrorKind::TimedOut => (ErrorCode::TimedOut, ErrorKind::Interruption),
        UseCaseErrorKind::Validation => (ErrorCode::InvalidArgument, ErrorKind::Validation),
        UseCaseErrorKind::Runtime => (ErrorCode::RuntimeFailure, ErrorKind::Runtime),
        UseCaseErrorKind::AgentCommandAbandoned => {
            (ErrorCode::AgentCommandAbandoned, ErrorKind::Platform)
        }
        UseCaseErrorKind::Platform => (ErrorCode::PlatformFailure, ErrorKind::Platform),
    }
}

#[cfg(test)]
mod tests {
    use super::with_cli_error;
    use crate::command_envelope::Envelope;
    use crate::platform::agent::AgentError;
    use crate::platform::process::ProcessError;
    use crate::support::error::AppError;
    use crate::use_cases::result::UseCaseError;
    use serde_json::Value;

    /// `error` конверта CLI для данного отказа — так, как его прочтёт потребитель.
    fn envelope_error(error: AppError) -> Value {
        let error = UseCaseError::from(error);
        let envelope = with_cli_error(Envelope::err("dump", 0, Value::Null), &error);
        serde_json::to_value(envelope).expect("envelope serializes")["error"].clone()
    }

    fn io() -> std::io::Error {
        std::io::Error::from(std::io::ErrorKind::ConnectionRefused)
    }

    /// Каждый отказ окружения на пути агента называет свой слой полем `layer` — литералом из
    /// закрытого набора, а не прозой (`INV.WIRE.AN-ENVIRONMENT-REFUSAL-NAMES-ITS-LAYER`).
    /// Таблица литеральная: перестановка варианта в другую ветку её валит.
    #[test]
    fn every_environment_refusal_of_the_agent_names_its_layer_in_the_cli_envelope() {
        let endpoint = || "127.0.0.1:1543".to_owned();
        let cases: Vec<(AgentError, &str)> = vec![
            (
                AgentError::AuthenticationRejected {
                    endpoint: endpoint(),
                    user: String::new(),
                },
                "infobase_user",
            ),
            (
                AgentError::Unreachable {
                    endpoint: endpoint(),
                    source: io(),
                },
                "agent_endpoint",
            ),
            (
                AgentError::Handshake {
                    endpoint: endpoint(),
                    source: russh::Error::Disconnect,
                },
                "agent_endpoint",
            ),
            (
                AgentError::HostKeyRejected {
                    endpoint: endpoint(),
                    expected: "a".to_owned(),
                    presented: "b".to_owned(),
                },
                "agent_endpoint",
            ),
            (
                AgentError::ForeignAgentOnPort {
                    endpoint: endpoint(),
                    expected: "a".to_owned(),
                    presented: "b".to_owned(),
                },
                "agent_endpoint",
            ),
            (
                AgentError::Channel {
                    endpoint: endpoint(),
                    source: russh::Error::Disconnect,
                },
                "agent_endpoint",
            ),
            (
                AgentError::Launch(ProcessError::SpawnFailed {
                    cmd: "1cv8 DESIGNER /AgentMode".to_owned(),
                    source: io(),
                }),
                "agent_start",
            ),
            (
                AgentError::PortTaken {
                    port: 1543,
                    detail: "taken".to_owned(),
                },
                "agent_start",
            ),
            (AgentError::NoFreePort { source: io() }, "agent_start"),
            (
                AgentError::StartupTimedOut {
                    timeout_ms: 5,
                    last: "x".to_owned(),
                },
                "agent_start",
            ),
            (
                AgentError::ExitedBeforeSession {
                    status: "0".to_owned(),
                    out: String::new(),
                },
                "agent_start",
            ),
            (
                AgentError::LiveAgentGone {
                    detail: "closing".to_owned(),
                },
                "agent_start",
            ),
            (
                AgentError::Keeper {
                    detail: "busy".to_owned(),
                },
                "agent_start",
            ),
            (
                AgentError::KeeperUnreachable {
                    channel: "pipe".to_owned(),
                    source: io(),
                },
                "agent_start",
            ),
            (
                AgentError::HostKeyPathNotReadable { tried: Vec::new() },
                "agent_start",
            ),
        ];
        for (error, layer) in cases {
            let shown = error.to_string();
            let rendered = envelope_error(AppError::from(error));
            assert_eq!(rendered["code"], "environment_unavailable", "{rendered}");
            assert_eq!(rendered["layer"], layer, "{shown}");
            // Текст остаётся уликой и не меняется: по нему опознают слой раннеры без поля.
            assert_eq!(rendered["message"], shown.as_str(), "{rendered}");
        }
    }

    /// Утилиты нет — слой `platform`; выпуска формата исходников нет — свой код и без поля.
    #[test]
    fn a_missing_utility_names_the_platform_layer_and_a_missing_release_names_none() {
        use crate::domain::platforms::SourceFormatInfo;
        use crate::platform::locator::{
            LocatorError, PlatformReleaseMissing, RequiredRelease, UtilityType,
        };

        let missing = envelope_error(
            AppError::from(LocatorError::NotFound {
                utility: UtilityType::Ibcmd,
                detail: None,
            })
            .with_context("load"),
        );
        assert_eq!(missing["code"], "environment_unavailable", "{missing}");
        assert_eq!(missing["layer"], "platform", "{missing}");

        let release = envelope_error(AppError::from(LocatorError::PlatformReleaseMissing(
            Box::new(PlatformReleaseMissing {
                required: RequiredRelease {
                    releases: Vec::new(),
                    source_format: SourceFormatInfo {
                        version: Some("2.21".to_owned()),
                        file: "/project/src/Configuration.xml".into(),
                        release: None,
                    },
                    unreadable: None,
                },
                installations: Vec::new(),
            }),
        )));
        assert_eq!(release["code"], "platform_release_missing", "{release}");
        assert!(release.get("layer").is_none(), "{release}");
    }

    /// Уточнение текста слоя не теряет — ни у ошибки раннера, ни у отказа сценария: так
    /// `push; agent at …` остаётся `infobase_user`.
    #[test]
    fn the_layer_survives_a_context() {
        let rejected = || {
            AppError::from(AgentError::AuthenticationRejected {
                endpoint: "127.0.0.1:1543".to_owned(),
                user: "Admin".to_owned(),
            })
        };
        let wrapped = envelope_error(rejected().with_context("push").with_context("apply"));
        assert_eq!(wrapped["layer"], "infobase_user", "{wrapped}");
        assert!(
            wrapped["message"]
                .as_str()
                .is_some_and(|message| message.starts_with("apply; push; agent at ")),
            "{wrapped}"
        );

        let refusal = UseCaseError::from(rejected())
            .with_context("push")
            .followed_by("check the credentials")
            .with_message("replaced");
        assert_eq!(
            refusal.environment_layer(),
            Some(crate::support::error::EnvironmentLayer::InfobaseUser)
        );
    }

    /// Отказ окружения, слой которого не определён, поля не несёт: его отсутствие и значит
    /// «не определён».
    #[test]
    fn an_environment_refusal_of_no_layer_carries_no_field() {
        let rendered = envelope_error(AppError::environment("services are Windows-only"));
        assert_eq!(rendered["code"], "environment_unavailable", "{rendered}");
        assert!(rendered.get("layer").is_none(), "{rendered}");
        let platform = envelope_error(AppError::Platform("agent command failed".to_owned()));
        assert!(platform.get("layer").is_none(), "{platform}");
    }
}
