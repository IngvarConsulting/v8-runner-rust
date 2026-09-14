---
id: CTR.WIRE.COMMAND-ENVELOPE
status: active
governs: product
version: 1
decision: DEC.2026-04-20.JSON-MESSAGE-IS-THE-ONLY-FORMAT-SWITCH
producer: src/command_envelope.rs
consumers: [cli, mcp, ci]
check: tests/cli_bootstrap.rs::action_logging_failure_in_json_mode_keeps_command_identity
scope: [wire, cli]
---

# Конверт ответа команды

Структурный ответ любой команды имеет одну форму: имя команды, статус, содержимое и ошибка. Имя команды в конверте не теряется даже тогда, когда сама команда не дошла до работы.
