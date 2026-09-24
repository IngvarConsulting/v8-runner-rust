---
id: INV.CLI.LOCK-BOUNDARY-IS-THE-ADAPTER
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/290
---

# Блокировку берёт адаптер команды, а не сценарий

Публичная команда, работающая с состоянием под `workPath`, захватывает блокировку на границе адаптера; вложенные шаги идут под ней.

Сегодня без замка идут `clone` и EDT-проверка по MCP в интерактивном режиме. Сторож
`tests/architecture_guardrails.rs::public_command_adapters_keep_workspace_lock_boundary`
сверяет перечень адаптеров, записанный руками, и этих мест не видит; он вернётся в `check`,
когда перечень перестанет быть ручным.
