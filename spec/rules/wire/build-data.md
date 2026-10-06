---
id: CTR.WIRE.BUILD-DATA
version: 3
artifact: docs/schemas/command-data/push.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - tests/mcp_stdio.rs::mcp_stdio_tools_answer_in_the_forms_of_their_commands
---

# `data` команды `build`

Сборка идёт по наборам исходников, и форма отчитывается по каждому: какой набор, каким
режимом загружен и почему. Режим выбирают правила частичной загрузки, а не вызывающий,
поэтому `mode` в ответе — решение раннера, и в нём же его причина.

Инструмент MCP `build_project` отвечает этой же формой: поверхность MCP не повторяет CLI,
но предмет команды у них один.

**Что изменила версия 3.** Квитанция `provider` получила необязательное поле
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
  "duration_ms": 1,
  "steps": [
    {
      "source_set": "main",
      "mode": "full",
      "ok": true,
      "message": "full load selected by partial-load rules; planned, Designer not dispatched",
      "duration_ms": 0
    }
  ]
}
```
