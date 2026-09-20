---
id: INV.PLATFORM.COMMAND-DISPLAY-HAS-ONE-OWNER
status: active
governs: process
decision: DEC.2026-09-16.A-RENDERED-COMMAND-MASKS-SECRETS-AT-ONE-PLACE
check: tests/architecture_guardrails.rs::a_process_command_is_shown_only_through_the_secrets_owner
scope: [platform]
---

# У показа команды один владелец

Модуль, показывающий argv запрошенного процесса — `ProcessRequest` или
`InteractiveProcessRequest`, в тексте отказа или в строке журнала, — берёт строку у
`platform::secrets` и не склеивает её сам. Второй составитель означал бы второе правило
маскирования, о котором первое не знает.

Правило говорит об argv. Показ, составленный не из запроса процесса — имя команды CLI,
адрес интерактивной сессии EDT, — под него не подпадает: это другой язык, и ключей
платформы в нём нет.
