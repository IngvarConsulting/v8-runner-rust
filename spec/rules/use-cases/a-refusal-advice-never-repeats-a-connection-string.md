---
id: INV.USE-CASES.A-REFUSAL-ADVICE-NEVER-REPEATS-A-CONNECTION-STRING
check:
  - src/use_cases/context.rs::an_advice_never_repeats_a_connection_string
  - tests/cli_dump.rs::a_command_line_advice_never_repeats_the_connection_string
  - tests/cli_dump.rs::an_mcp_advice_never_repeats_the_connection_string_of_the_server
---

# Совет отказа не повторяет строку соединения

Команда, которую советует отказ, называет базу только именем, объявленным в проекте. Если
`--infobase` вызова или сервера MCP — строка соединения, совет просит то же значение
`--infobase` словами и самой строки не содержит ни в каком транспорте.

Строка соединения может нести секрет, которого загрузчик не отвергает (`Wsp=`, сегмент без
`=`), а совет показывают, пишут в журнал и копируют в чужую оболочку. Маскирование здесь
не годится: замаскированная строка в готовой команде всё равно не выполнилась бы.
