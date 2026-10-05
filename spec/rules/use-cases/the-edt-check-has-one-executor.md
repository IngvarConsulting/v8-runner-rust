---
id: INV.USE-CASES.THE-EDT-CHECK-HAS-ONE-EXECUTOR
check:
  - tests/architecture_guardrails.rs::the_edt_project_check_has_one_executor
  - tests/architecture_guardrails.rs::the_edt_check_finder_sees_a_second_executor_under_another_name
  - src/use_cases/check_syntax.rs::the_server_session_runs_the_same_executor
---

# Проверку проекта EDT выполняет единственный исполнитель

Проверку проекта EDT — `check` для формата EDT и инструмент MCP `check_syntax_edt` —
выполняет сценарий `use_cases::check_syntax`, и никакой другой модуль не запускает
`validate` EDT и не читает её журнал. Транспорт выбирает только общую сессию EDT и способ
её ждать: командная строка держит свою сессию, сервер MCP отдаёт свою.
