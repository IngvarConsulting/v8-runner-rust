---
id: INV.WIRE.AN-ENVIRONMENT-REFUSAL-NAMES-ITS-LAYER
check:
  - src/cli/output.rs::every_environment_refusal_of_the_agent_names_its_layer_in_the_cli_envelope
  - src/cli/output.rs::a_missing_utility_names_the_platform_layer_and_a_missing_release_names_none
  - src/cli/output.rs::the_layer_survives_a_context
  - src/cli/output.rs::an_environment_refusal_of_no_layer_carries_no_field
  - src/command_envelope.rs::the_environment_layer_set_is_closed_and_named_by_the_schema
  - src/platform/agent_keeper/protocol.rs::an_environment_layer_and_a_kind_survive_the_keeper_channel
  - src/mcp/error.rs::an_agent_refusal_names_its_layer_over_mcp
  - src/mcp/service.rs::the_mcp_response_envelope_keeps_the_environment_layer
  - src/use_cases/agent_session.rs::a_keeper_that_did_not_start_is_agent_start_and_a_busy_agent_names_no_layer
  - src/use_cases/provider_selection.rs::nobody_ready_without_a_utility_names_the_platform_layer
  - tests/cli_dump_agent.rs::a_rejected_password_is_an_environment_refusal_even_though_the_port_answers
  - tests/cli_dump_agent.rs::an_unreachable_attached_agent_is_refused_and_no_process_is_launched_instead
  - tests/cli_dump_agent.rs::a_managed_agent_without_a_declared_key_refuses_a_foreign_key_on_its_port
  - tests/cli_infobase.rs::a_taken_port_of_the_managed_agent_is_named
  - tests/cli_dump_agent.rs::a_managed_agent_without_the_local_platform_is_refused_before_any_connection
  - tests/cli_convert.rs::convert_without_the_edt_cli_answers_an_environment_failure
  - tests/cli_infobase.rs::missing_file_infobase_is_unavailable_before_designer_dispatch
  - tests/cli_dbms.rs::other_systems_refuse_start_as_the_environment
---

# Отказ окружения называет слой, который отказал

Отказ `environment_unavailable` несёт поле `error.layer` рядом с `code` и `message`, когда
известно, какой слой окружения отказал. Потребитель решает по полю, а не по тексту: текст
остаётся уликой человеку и прежним, чтобы раннеры без поля опознавались, как раньше.

Набор значений закрыт:

| `layer` | Что отказало | Отказы |
| --- | --- | --- |
| `infobase_user` | агент отверг вход с учётными данными пользователя базы | `AgentError::AuthenticationRejected` |
| `agent_endpoint` | точка входа агента: не ответила, рукопожатие SSH не прошло, ключ хоста не тот | `Unreachable`, `Handshake`, `HostKeyRejected`, `ForeignAgentOnPort`, `Channel` |
| `agent_start` | агент не поднялся | `Launch`, `PortTaken`, `NoFreePort`, `StartupTimedOut`, `ExitedBeforeSession`, `LiveAgentGone`, `Keeper`, `KeeperUnreachable`, `HostKeyPathNotReadable`; нет каталога состояния учётной записи, хранитель не назвал порт, хранитель не поднялся, `host-key` не в ASCII |
| `platform` | утилиты платформы нет, она не той версии или не выбрана | `AppError::PlatformLocator*`; управляемому агенту не выбран `1cv8`; не готов ни один исполнитель, и хоть одному не нашлось утилиты |

`agent_endpoint` — любая точка входа агента: чужая, шлюз автономного сервера и порт агента,
которого поднял сам раннер. `infobase_user` получает и агент без лицензии платформы: он
отвергает вход так же, как неверный пароль (замер в
`references/1c/confirmed-runtime-measurements.md`, 8.5.4.1878).

Поля нет — слой не определён. Так отвечают остальные отказы `environment_unavailable`:
службы не на Windows, реестр живых агентов, живой агент, занятый другой командой у
`agent hold`, ссылка в каталоге агента, `diff` без исполняемого файла, выбор исполнителя,
где все кандидаты пропущены не из-за утилиты — нет файла базы, не объявлен путь. Поля нет и у
`platform_release_missing`: у него свой код.

Слой переживает уточнение текста (`with_context`) и канал хранителя живого агента: живой
агент или разовый — слой тот же. MCP несёт то же поле в отказе (`McpBusinessError.layer`) и в
конверте ответа, хотя код у него `runtime_failure`.
