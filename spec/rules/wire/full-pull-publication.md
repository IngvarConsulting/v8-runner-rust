---
id: CTR.WIRE.FULL-PULL-PUBLICATION
version: 3
artifact: docs/schemas/full-pull-publication.schema.json
check:
  - src/cli/external_pull.rs::generated_publication_schema_is_current
  - tests/cli_external_full_pull.rs::accepted_external_full_pull_remembers_the_original_source_for_the_next_push
  - tests/cli_external_full_pull.rs::invalid_decisions_after_ready_are_unknown_and_do_not_commit_memory
---

# Полная выгрузка согласует внешнюю публикацию по JSONL

`pull <SET> --force --publication-stdio <STAGE>` в отдельном процессе передаёт
кадры JSONL через stdout и принимает ответы через stdin. Форма закрыта схемой;
ответ повторяет `version`, уникальный для запуска `nonce` и ожидаемую `phase`.
Ответ ограничен 4096 байтами вместе с переводом строки. Неверный ответ
не разрешает следующий шаг.

`selected` называет квитанцию выбора провайдера и фактического исполнителя:
`kind` (`local`, `attached`, `remote`), `executable`, `endpoint`. До ответа
`continue` к платформе не обращаются; `reject` завершает запрос. Выбранный
исполнитель после согласования сохраняется. Для исполнения вызывающий готовит
родительский каталог цели до `continue`, чтобы раннер удерживал общий замок
выгрузки. Отсутствующий родитель даёт отказ до обращения к платформе; превью
такой подготовки не требует.

`ready` передаёт подготовленный результат: `command`, `requested_mode`, `mode`,
`mode_reason`, `extension`, `provider_dispatched`, `source_set`, итоговый
`target_path`, `stage_path`, готовность снимка `snapshot` (`prepared`,
`unavailable`) и `snapshot_warning`. Поля результата соответствуют выбранной
операции и фактически выполненной работе до публикации. На Windows `target_path` и
`stage_path` могут нести приставку `\\?\`: это операнды публикации, а не показ
([правило](a-response-names-a-path-without-the-verbatim-prefix.md)).
`published` подтверждает уже выполненную вызывающим публикацию; `rejected`
подтверждает отказ от неё. Превью заканчивается после согласования исполнителя.

Терминальный кадр содержит обычный конверт команды в `result` и состояния
`publication` (`not_published`, `published`, `unknown`) и `memory`
(`unchanged`, `saved`, `not_saved`). После передачи `ready` потеря связи,
неверный ответ или отмена до подтверждения дают `unknown` и `unchanged`.
Подтверждённая публикация сохраняет `published`, в том числе при ошибке записи
памяти. Состояние памяти называет результат её локального завершения.

**Что изменила версия 3.** Перечень значений `mode_reason` тот же, что у
`CTR.WIRE.PULL-DATA` версии 9: добавилось `platform_rewrote_all`. Внешний полный pull —
полная выгрузка без обновления, перечня по факту не делает и этой причины сам не называет; значение появилось в схеме, потому что перечень
у форм общий. Код отказа `no_memory` в этой версии называется `untracked_base`. Конверт в
`result` знает и код `platform_release_missing` с полем отказа `platform_release`
(`CTR.WIRE.COMMAND-ENVELOPE` версии 5).

**Что изменила версия 2.** Квитанция выбора несёт необязательное `reason` формы
`CTR.WIRE.PULL-DATA` версии 8: полная выгрузка файловой базы без ключа `providers.pull` называет
`whole`.

## Пример

```json
{"version":1,"nonce":"fb9824d8-6c77-4219-86b6-54f935a83c8e","phase":"ready","decision":"published"}
```
