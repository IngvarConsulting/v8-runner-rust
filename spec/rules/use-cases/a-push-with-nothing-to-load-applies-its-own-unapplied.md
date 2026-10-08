---
id: INV.USE-CASES.A-PUSH-WITH-NOTHING-TO-LOAD-APPLIES-ITS-OWN-UNAPPLIED
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/210
---

# Отправка без изменений применяет своё непринятое

`push` делает и загрузку, и применение (`INV.CLI.APPLY-IS-A-SEPARATE-STEP`). Если загружать
нечего, а запись набора помечена «загружено, не применено» и поколение базы ей равно,
отправка запускает исполнителя только для применения. Так `test` и инструмент MCP
`build_project` не запускают клиента на базе с прежней конфигурацией базы данных.
