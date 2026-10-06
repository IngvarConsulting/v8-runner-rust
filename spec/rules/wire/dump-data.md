---
id: CTR.WIRE.DUMP-DATA
version: 4
artifact: docs/schemas/command-data/pull.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - tests/mcp_stdio.rs::mcp_stdio_tools_answer_in_the_forms_of_their_commands
---

# `data` команды `dump`

Выгрузка базы обратно в файлы проекта отчитывается тем, куда легли файлы и каким режимом:
полным, инкрементальным или частичным. У частичного форма дополнительно называет
селекторы — и запрошенный, и приведённый к каноническому виду, потому что раннер их
нормализует и вызывающий обязан видеть результат нормализации.

Инструмент MCP `dump_config` отвечает этой же формой.

**Что изменила версия 4.** Квитанция `provider` получила необязательное поле
`endpoint` — точку входа сессии агента, через которую шло исполнение: `mode`
(`managed`, `attached` или `gate`) и `address` — `host:port` подключения без учётных
данных. Поле есть, только когда команда открыла сессию агента; у процесса платформы,
у превью и у отказа до подключения его нет ([правило](a-session-receipt-names-its-endpoint.md)). Прежде квитанция
точку входа не называла.

## Пример

```json
{
  "provider": {"selected": "designer", "origin": {"kind": "default"}},
  "ok": true,
  "provider_dispatched": false,
  "source_set": "main",
  "mode": "FULL",
  "target_path": "src/cf",
  "duration_ms": 0,
  "message": "would dump Full into 'src/cf' via /opt/1cv8/bin/1cv8; nothing written"
}
```
