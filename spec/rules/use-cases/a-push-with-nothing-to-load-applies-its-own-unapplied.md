---
id: INV.USE-CASES.A-PUSH-WITH-NOTHING-TO-LOAD-APPLIES-ITS-OWN-UNAPPLIED
check:
  - tests/cli_apply.rs::a_push_with_nothing_to_load_applies_its_own_unapplied
---

# Отправка без изменений применяет своё непринятое

`push` делает и загрузку, и применение (`INV.CLI.APPLY-IS-A-SEPARATE-STEP`). Если загружать
нечего, а запись набора помечена «загружено, не применено» и поколение базы ей равно,
отправка запускает исполнителя только для применения. Так `test` и инструмент MCP
`build_project` не запускают клиента на базе с прежней конфигурацией базы данных.
