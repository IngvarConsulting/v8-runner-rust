---
id: INV.PLATFORM.A-LAUNCH-PARAMETER-REACHES-THE-CLIENT-AS-ONE-VALUE
check:
  - src/platform/enterprise.rs::typed_application_parameter_reaches_the_client_as_exactly_one_value
  - src/platform/enterprise.rs::typed_application_parameter_matches_the_measured_macos_argv
  - src/platform/enterprise.rs::credentials_spelled_like_the_launch_key_do_not_take_the_verbatim_slot
  - src/platform/enterprise.rs::enterprise_dsl_passes_the_verbatim_index_to_the_runner
  - src/use_cases/launch_app.rs::client_requests_keep_the_verbatim_launch_parameter
  - src/platform/process.rs::windows_verbatim_args_reach_the_child_command_line_unescaped
---

# Значение `/C` приходит в клиент одним параметром

Значение `/C`, которое раннер передаёт клиенту 1С, — из `launch --c`, `launch mcp`,
`test yaxunit` и `test va`, — доходит до `ПараметрЗапуска` ровно одной строкой, какие
бы кавычки, пробелы, табуляции, переводы строк и ключи вида `/Execute` в нём ни были.
Часть значения не становится другим ключом запуска: внешняя обработка открывается
только явным `--execute` отдельным аргументом со своими проверками.

Раннер кодирует значение под разбор командной строки самой платформой: внутри кавычек
`""` — одна кавычка. На Windows клиент читает исходную командную строку, поэтому
значение всегда обрамляется и передаётся как есть, мимо экранирования `Command::arg`.
На macOS и Linux платформа собирает строку из argv сама и обрамляет лишь аргумент
с ASCII-пробелом; любое другое значение — пустое, с табуляцией или переводом строки —
обрамляет раннер.

Место значения в аргументах раннер знает от того, кто его выставил, а не ищет по
тексту: имя пользователя или пароль `/C` его не подменят.

Живая проверка — macOS, 8.3.27, в `references/1c/confirmed-runtime-measurements.md`.
Linux и Windows живьём не замерены. Для Linux принято, что платформа ведёт себя как
на macOS. Для Windows принято, что клиент разбирает исходную строку с `""`; тест
подтверждает только, что значение доходит до командной строки без экранирования.
