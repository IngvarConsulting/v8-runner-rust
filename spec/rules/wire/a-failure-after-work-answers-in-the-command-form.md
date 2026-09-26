---
id: INV.WIRE.A-FAILURE-AFTER-WORK-ANSWERS-IN-THE-COMMAND-FORM
check:
  - tests/contract_dispatch.rs::the_flag_says_whether_a_stub_executor_ran
  - tests/cli_publish.rs::publish_interrupted_after_webinst_started_answers_in_its_form
  - tests/cli_launch.rs::an_epf_wait_interrupted_after_the_client_started_answers_in_its_form
  - tests/cli_extensions.rs::extensions_info_that_fails_after_the_platform_ran_answers_in_its_form
  - tests/cli_agent_scenarios.rs::extensions_list_and_info_through_the_agent_read_the_structured_reply
  - tests/mcp_stdio.rs::mcp_stdio_a_project_that_misses_the_edt_session_after_work_answers_in_the_check_form
  - src/mcp/edt_syntax.rs::a_project_that_misses_the_session_answers_by_the_work_mark
  - src/use_cases/result.rs::a_failure_answers_in_the_command_form_only_after_work
  - src/use_cases/result.rs::the_stamp_catches_a_failure_without_its_form_after_work
---

# Отказ после работы исполнителя отвечает формой команды

Когда исполнитель уже получил работу команды, её отказ отвечает формой `data` самой команды:
у CLI — конвертом с этой формой, у MCP — результатом инструмента в этой форме. Ни общая форма
отказа CLI или MCP, ни ошибка протокола такому отказу не годятся: вызывающий, который решает,
повторять ли команду, узнаёт из формы, что работа уже была.

Очередь общей сессии EDT внутри допущенного MCP-вызова — не исключение. Проект, который не
дождался сессии после того, как предыдущий проект уже проверялся, отказывает формой `check`.
