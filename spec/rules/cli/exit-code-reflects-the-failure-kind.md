---
id: INV.CLI.EXIT-CODE-REFLECTS-THE-FAILURE-KIND
check: [src/use_cases/result.rs::use_case_error_kinds_keep_stable_cli_exit_codes]
---

# Код выхода различает отказ сценария и сбой обвязки

Ожидаемый отказ сценария и сбой обвязки дают разные коды выхода: вызывающий отличает «не получилось» от «сломалось» без чтения текста.
