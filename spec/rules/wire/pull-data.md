---
id: CTR.WIRE.PULL-DATA
version: 5
artifact: docs/schemas/command-data/pull.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - tests/mcp_stdio.rs::mcp_stdio_tools_answer_in_the_forms_of_their_commands
---

# `data` команды `pull`

Выгрузка базы обратно в файлы проекта отчитывается тем, куда легли файлы и каким режимом:
полным, инкрементальным или частичным. У частичного форма дополнительно называет
селекторы — и запрошенный, и приведённый к каноническому виду, потому что раннер их
нормализует и вызывающий обязан видеть результат нормализации.

Инструмент MCP `dump_config` отвечает этой же формой.

**Что изменила версия 5.** Появилось необязательное поле `losses` — перечень того, что в
каталоге набора пропадает безвозвратно, каждым путём: после выгрузки с согласием
(`--force`) — уничтоженное, у превью — что выгрузка уничтожила бы или на чём остановилась бы
без согласия. Пути гит называет от корня рабочей копии; где он не ответил, путь полный и
потерей считается каждый файл каталога. Когда терять нечего, поля нет
([правило](../use-cases/force-names-what-it-destroyed.md)). Прежде ответ уничтоженного не
называл.

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
  "message": "would dump Full into 'src/cf' via /opt/1cv8/bin/1cv8; nothing written; it would discard in 'src/cf': 1 file(s) there exist nowhere else (src/cf/hand-written.xml)",
  "losses": ["src/cf/hand-written.xml"]
}
```
