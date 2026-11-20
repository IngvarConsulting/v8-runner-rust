---
id: CTR.WIRE.INFOBASE-CHECK-DATA
version: 1
artifact: docs/schemas/command-data/infobase-check.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_infobase.rs::infobase_check_needs_native_login_evidence_and_leaves_no_files
  - tests/mcp_stdio.rs::mcp_check_infobase_confirms_login_and_reports_a_missing_file
---

# Проверка входа в базу

CLI `infobase check` и MCP `check_infobase` отвечают `command: "infobase.check"`.
Успех подтверждает вход: выбранный Конфигуратор вернул код 0 и действительный
идентификатор поколения. `data.platform_version` — точная версия этого клиента 1С,
а не версия кластера. Наборы исходников и список расширений для проверки не нужны.
Если версия выбранного исполняемого файла неизвестна, проверка отказывает до запуска.

`infobase_not_found` означает, что для файловой цели ОС подтвердила отсутствие файла
базы. При отказе запуска сеанса без подтверждённой причины код `connection_failed`.
Отказ аутентификации нельзя выводить из локализованного журнала или общего кода выхода.
Отмена, тайм-аут и неверный формат ответа имеют собственные коды CLI. MCP
сохраняет общий код `platform_failure` для этих отказов. При отказе после запуска `data` —
пустой объект. Локализованная диагностика может сопровождать отказ как улика.
Приватный каталог проверки в `workPath/temp` убирается перед ответом.

Защита от повторения: сырой адрес рабочего проекта может содержать управляющие
ключи Конфигуратора. Допустимость адреса для пробы определяет
`use_cases::infobase_check::check`: только один ключ адреса с его значением,
реквизиты берутся отдельно. Проверка CLI подставляет лишнюю команду платформы
и требует отказ до запуска.

## Пример

```json
{"platform_version": "8.3.27.2074"}
```
