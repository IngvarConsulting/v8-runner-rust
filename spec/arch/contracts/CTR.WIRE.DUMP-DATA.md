---
id: CTR.WIRE.DUMP-DATA
status: active
governs: product
version: 1
decision: DEC.2026-09-14.EVERY-COMMAND-PINS-THE-FORM-OF-ITS-DATA
artifact: docs/schemas/command-data/dump.schema.json
producer: src/domain/dump.rs
consumers: [cli, mcp, unica]
check: [src/command_data.rs::generated_command_data_schemas_are_current, tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it]
scope: [wire, cli, mcp]
---

# `data` команды `dump`

Выгрузка базы обратно в файлы проекта отчитывается тем, куда легли файлы и каким режимом:
полным, инкрементальным или частичным. У частичного форма дополнительно называет
селекторы — и запрошенный, и приведённый к каноническому виду, потому что раннер их
нормализует и вызывающий обязан видеть результат нормализации.

Инструмент MCP `dump_config` отвечает этой же формой.

## Пример

```json
{
  "ok": true,
  "provider_dispatched": false,
  "source_set": "main",
  "mode": "FULL",
  "target_path": "src/cf",
  "duration_ms": 0,
  "message": "would dump Full into 'src/cf' via /opt/1cv8/bin/1cv8; nothing written"
}
```
