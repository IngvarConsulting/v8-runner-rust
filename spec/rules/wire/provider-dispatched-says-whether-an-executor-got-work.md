---
id: INV.WIRE.PROVIDER-DISPATCHED-SAYS-WHETHER-AN-EXECUTOR-GOT-WORK
check:
  - tests/contract_dispatch.rs::the_flag_says_whether_a_stub_executor_ran
  - tests/contract_dispatch.rs::no_form_says_an_executor_got_work_when_there_is_none
  - tests/contract_dispatch.rs::every_form_carrying_the_flag_has_a_row_with_work
  - tests/contract_previews.rs::no_preview_claims_that_an_executor_got_work
  - src/platform/process.rs::only_a_started_process_marks_the_work
  - src/platform/process.rs::a_process_cancelled_after_its_start_still_got_the_work
  - src/platform/process.rs::a_managed_process_marks_the_work_once_started
  - src/platform/browser.rs::only_a_started_opener_marks_the_work
  - tests/cli_launch.rs::launch_mcp_wait_ready_fails_when_endpoint_never_starts
  - src/platform/edt_session.rs::only_a_delivered_work_request_marks_the_work
  - src/platform/edt_session.rs::a_work_request_cancelled_during_the_baseline_marks_nothing
  - src/platform/edt_session.rs::a_request_refused_by_a_full_queue_marks_nothing
  - src/platform/edt.rs::an_interactive_session_marks_only_the_request_command
  - tests/mcp_stdio.rs::mcp_stdio_tools_answer_in_the_forms_of_their_commands
  - tests/mcp_stdio.rs::mcp_stdio_the_live_edt_check_answers_in_the_form_of_check
  - tests/mcp_stdio.rs::mcp_stdio_a_live_edt_check_whose_session_never_started_reports_no_work
  - tests/cli_agent_scenarios.rs::a_server_base_without_dbms_serves_extensions_through_the_agent
  - tests/cli_agent_scenarios.rs::an_agent_session_without_a_request_command_gives_no_work
  - tests/cli_dump_agent.rs::an_attached_agent_released_without_a_request_command_gives_no_work
  - src/use_cases/load_artifact.rs::a_configuration_probe_cancelled_after_its_start_reports_the_work
  - src/use_cases/load_artifact.rs::an_extension_probe_cancelled_after_its_start_reports_the_work
  - src/use_cases/load_artifact.rs::a_load_refused_before_its_first_process_reports_no_work
  - tests/cli_extensions.rs::extensions_command_without_targets_does_not_read_as_a_preview
  - tests/architecture_guardrails.rs::provider_dispatched_takes_its_value_only_from_the_work_mark
  - tests/architecture_guardrails.rs::a_step_may_skip_the_work_mark_only_inside_the_platform
---

# `provider_dispatched` говорит, получил ли исполнитель работу

Где форма `data` несёт `provider_dispatched`, признак говорит, получил ли исполнитель работу
этой команды. Исполнитель здесь — любая программа, которой команда отдаёт работу: платформа
(Конфигуратор, `ibcmd`, агент), EDT CLI, `webinst`, клиент 1С или системная программа,
открывающая адрес у `launch web`. `true` — запущен процесс, который её выполняет, либо работающей сессии — общей
сессии EDT или сессии агента — отдана команда запроса. Подъём и открытие сессии, в том числе
запуск её процесса, и её служебные команды — подключение к базе, сброс базового состояния,
закрытие — работой не считаются. `false` — работы исполнитель не получил, чем бы команда ни
кончилась: превью, отказ или прерывание до передачи работы, прогон, которому исполнитель не
понадобился, сессия без команды запроса, процесс, который не удалось запустить.

Превью признак не опознаёт: о превью вызывающий знает из своего запроса.
