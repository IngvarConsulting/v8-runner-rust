---
id: CTR.WIRE.PULL-DATA
version: 6
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

**Что изменила версия 6.** Ответ называет и запрошенный режим (`requested_mode`), и
случившийся (`mode`): по плану раннера и по прогнозу платформы
([правило](../use-cases/the-dump-mode-is-forecast-in-the-same-command.md)). У `mode` новое
значение `UNKNOWN` — прогноз не распознан или не получен. Необязательное `mode_reason` говорит,
почему случившийся режим не тот, что просили: `version_file` — файла версий нет или он не
распознан, `foreign_format` — версия формата не та, что пишет платформа, `platform_forecast` —
платформа предсказала полную выгрузку, `unknown` — режим не известен. Когда режим тот, что
просили, поля нет. Выборка `ibcmd` теперь называет случившимся режимом `INCREMENTAL` без
`mode_reason`: объекты она выбирать не умеет и выгружает изменившееся, о чём говорит
предупреждение в `message`. Прежде `mode` называл режим плана, а запрошенного
ответ не называл.

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
  "requested_mode": "FULL",
  "mode": "FULL",
  "target_path": "src/cf",
  "duration_ms": 0,
  "message": "would dump Full into 'src/cf' via /opt/1cv8/bin/1cv8; nothing written; it would discard in 'src/cf': 1 file(s) there exist nowhere else (src/cf/hand-written.xml)",
  "losses": ["src/cf/hand-written.xml"]
}
```
