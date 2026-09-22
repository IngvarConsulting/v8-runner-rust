---
id: CTR.WIRE.RESET-DATA
status: active
governs: product
version: 1
decision: DEC.2026-09-22.COMPATIBLE-CONFIGURATION-TRANSITIONS
artifact: docs/schemas/command-data/reset.schema.json
producer: src/cli/execute.rs
consumers: [cli, unica]
check: src/command_data.rs::generated_command_data_schemas_are_current
scope: [wire, cli]
---

# data команды reset

provider_dispatched различает false (не запускали), true (получен результат
процесса) и null (после ошибки исполнения факт запуска неизвестен). Completed
означает подтверждённое успешное завершение, а не предположение по запуску.
Preview имеет dry_run:true, provider_dispatched:false и completed:false.
Extension:null означает основную конфигурацию; строка — единственную цель.
Status, interruption и warnings сохраняют исход и отложенное прерывание.

## Пример

```json
{"duration_ms":0,"dry_run":true,"extension":null,"provider_dispatched":false,"completed":false,"status":"succeeded","provider":{"selected":"designer","origin":{"kind":"default"}},"platform_log_path":null,"interruption":null,"warnings":[]}
```
