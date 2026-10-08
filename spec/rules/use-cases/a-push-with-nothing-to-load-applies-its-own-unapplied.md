---
id: INV.USE-CASES.A-PUSH-WITH-NOTHING-TO-LOAD-APPLIES-ITS-OWN-UNAPPLIED
check:
  - tests/cli_apply.rs::a_push_with_nothing_to_load_applies_its_own_unapplied
  - src/use_cases/build_project.rs::execute_ibcmd_build_honors_interruption_before_apply_safe_point
---

# Отправка без изменений применяет своё непринятое

`push` делает и загрузку, и применение (`INV.CLI.APPLY-IS-A-SEPARATE-STEP`). Если загружать
нечего, а запись набора помечена «загружено, не применено» и поколение базы ей равно,
отправка запускает исполнителя только для применения. База ушла от записи — отправка не
применяет и называет непринятое с выходом `apply`. Загрузка, после которой поколение не
известно, памятью исходников не фиксируется: следующая отправка загрузит и применит набор
снова. Так `test` и инструмент MCP
`build_project` не запускают клиента на базе с прежней конфигурацией базы данных.
