---
id: INV.CLI.AN-INTERACTIVE-CLIENT-MAY-SHOW-STARTUP-DIALOGS
check:
  - src/use_cases/launch_app.rs::only_an_interactive_enterprise_client_may_show_startup_dialogs
  - tests/cli_launch.rs::launch_dry_run_json_returns_a_plan_without_dispatching_the_client
  - tests/cli_launch.rs::launch_ordinary_supports_typed_keys_and_filters_reserved_raw_duplicates
  - tests/cli_launch.rs::launch_mcp_va_builds_payload_from_configured_port_and_ordinary_mode
  - src/use_cases/launch_app.rs::designer_launch_app_does_not_append_enterprise_additional_keys
  - src/platform/enterprise.rs::enterprise_dsl_applies_internal_out_to_launch
  - tests/cli_launch_debug.rs::interactive_login_withholds_saved_credentials_and_keeps_the_launch_target
  - tests/cli_launch_debug.rs::interactive_login_rejects_unattended_launches_and_native_overrides
  - tests/cli_launch_debug.rs::interactive_login_refuses_inline_credentials_in_the_selected_address
---

# Интерактивный клиент предприятия может показать окно при старте

`launch thin`, `launch thick` и `launch ordinary` без `--wait-for-exit` запускают клиента
предприятия без `/DisableStartupDialogs`. Клиент показывает окна, которые платформа выводит
при старте, и на них отвечает человек: тонкий клиент на комьюнити-лицензии — окно лицензии,
каждый из этих клиентов — окно входа, если имя или пароль не подошли.

С ключом тонкий клиент на комьюнити-лицензии выходил с кодом 1 сразу после успешного входа:
ему нужно окно лицензии, а ключ окна запрещает (замер — раздел «Тонкий клиент на
комьюнити-лицензии и `/DisableStartupDialogs` (#784)» в
`references/1c/confirmed-runtime-measurements.md`).

Запуск, которого ждёт раннер или программа, ключ сохраняет: окну там отвечать некому.
`launch designer`, `launch mcp`, `launch thin --wait-for-exit`, клиент `test` и пакетные
вызовы Конфигуратора идут с `/DisableStartupDialogs`. `/DisableStartupDialogs` в `--raw-key` и
`tools.enterprise.additional-launch-keys` по-прежнему отбрасывается: решает вид запуска.

У интерактивных запусков `thin`, `thick` и `ordinary` ключ `--interactive-login` оставляет выбор пользователя
платформе на один запуск. Раннер опускает сохранённые `infobase.user` и `infobase.password`,
разрешает стартовые окна и передаёт `/WA-`, чтобы вход через учётную запись ОС не обходил
выбор пользователя. Адрес выбранной ИБ, платформа и `--debug` сохраняются. База без
пользователей открывается по штатному сценарию платформы. `--dry-run` строит тот же план.
Показ окна определяется настройками аутентификации 1С, включая сохранённые токены платформы.
Сырой адрес с реквизитами и сырые ключи, меняющие цель, платформу или вход, дают
`invalid_argument` до запуска. Обычные запуски и пакетные вызовы используют реквизиты профиля.
