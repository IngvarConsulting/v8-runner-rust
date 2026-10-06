---
id: INV.WIRE.THE-SHARED-EDT-SESSION-VERDICT-IS-READ-FROM-ITS-OUTPUT
check:
  - tests/cli_syntax.rs::the_shared_edt_session_verdict_on_the_command_line_is_the_servers
  - tests/mcp_stdio.rs::mcp_stdio_edt_syntax_treats_stdout_without_issues_as_tool_failure
  - tests/mcp_stdio.rs::mcp_stdio_edt_syntax_preserves_issues_found_when_stdout_is_non_empty
  - src/use_cases/check_syntax.rs::a_session_verdict_is_read_from_its_output
  - src/use_cases/check_syntax.rs::a_session_refusal_or_timeout_is_a_runtime_failure
---

# Вердикт проверки EDT в общей сессии читается по её выводу

У команды общей сессии EDT кода выхода нет, поэтому вердикт `check` читается по её выводу
и журналу — одинаково для командной строки с `interactive-mode: true` и для инструмента
MCP `check_syntax_edt`. Поток ошибок — `tool_failed`, даже если журнал назвал замечания.
Вывод в поток ответа без замечаний журнала — `tool_failed`, а не `clean`: приговора в нём
нет. Замечания журнала без потока ошибок — `issues_found`. Тишина с прочитанным журналом
без замечаний — `clean`.

`exit_code` здесь не наблюдался и выводится из вердикта: `0` у `clean`, `101` у
`issues_found`, `-1` у `tool_failed`. Отказ сессии и срок, истёкший у запроса в работе,
отвечают формой `check` со `status: tool_failed` и отказом рода `runtime`
(`runtime_failure`).
