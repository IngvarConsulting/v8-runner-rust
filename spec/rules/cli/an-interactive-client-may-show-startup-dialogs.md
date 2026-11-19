---
id: INV.CLI.AN-INTERACTIVE-CLIENT-MAY-SHOW-STARTUP-DIALOGS
check:
  - src/use_cases/launch_app.rs::only_an_interactive_enterprise_client_may_show_startup_dialogs
  - tests/cli_launch.rs::launch_dry_run_json_returns_a_plan_without_dispatching_the_client
  - tests/cli_launch.rs::launch_ordinary_supports_typed_keys_and_filters_reserved_raw_duplicates
  - tests/cli_launch.rs::launch_mcp_va_builds_payload_from_configured_port_and_ordinary_mode
---

# Интерактивный клиент предприятия может показать окно при старте

`launch thin`, `launch thick` и `launch ordinary` без `--wait-for-exit` запускают клиента
предприятия без `/DisableStartupDialogs`. Клиент показывает окна, которые платформа выводит
при старте: окно лицензии и окно входа, если имя или пароль не подошли. На них отвечает
человек.

С ключом тонкий клиент на комьюнити-лицензии выходил с кодом 1 сразу после успешного входа:
ему нужно окно лицензии, а ключ окна запрещает (замер — раздел «Исход входа интерактивного
клиента в журнале регистрации файловой базы (#777)» в
`references/1c/confirmed-runtime-measurements.md`, причина — #784).

Запуск, которого ждёт раннер или программа, ключ сохраняет: окну там отвечать некому.
`launch designer`, `launch mcp`, `launch thin --wait-for-exit`, клиент `test` и пакетные
вызовы Конфигуратора идут с `/DisableStartupDialogs`. `/DisableStartupDialogs` в `--raw-key` и
`tools.enterprise.additional-launch-keys` по-прежнему отбрасывается: решает вид запуска.
