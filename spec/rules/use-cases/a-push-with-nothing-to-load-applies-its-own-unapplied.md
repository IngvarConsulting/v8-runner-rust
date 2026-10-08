---
id: INV.USE-CASES.A-PUSH-WITH-NOTHING-TO-LOAD-APPLIES-ITS-OWN-UNAPPLIED
check:
  - tests/cli_apply.rs::a_push_with_nothing_to_load_applies_its_own_unapplied
  - tests/cli_apply.rs::a_push_with_nothing_to_load_names_an_unapplied_load_the_base_moved_away_from
  - tests/cli_apply.rs::a_push_with_nothing_to_load_applies_an_unapplied_designer_extension
---

# Отправка без изменений применяет своё непринятое

`push` делает и загрузку, и применение (`INV.CLI.APPLY-IS-A-SEPARATE-STEP`). Если загружать
нечего, а запись набора помечена «загружено, не применено» и поколение базы ей равно,
отправка запускает исполнителя только для применения. Так `test` и инструмент MCP
`build_project` не запускают клиента на базе с прежней конфигурацией базы данных. Если база
ушла от записи или запись сделана другим инструментом, отправка не применяет, а называет
непринятое и выход `apply`.
