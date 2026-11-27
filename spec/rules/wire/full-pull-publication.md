---
id: CTR.WIRE.FULL-PULL-PUBLICATION
version: 6
artifact: docs/schemas/full-pull-publication.schema.json
check:
  - src/cli/external_pull.rs::generated_publication_schema_is_current
  - tests/cli_external_full_pull.rs::accepted_external_full_pull_remembers_the_original_source_for_the_next_push
  - tests/cli_external_full_pull.rs::accepting_a_merge_baseline_preserves_outgoing_changes_and_refuses_a_moved_base
  - tests/cli_external_full_pull.rs::invalid_decisions_after_ready_are_unknown_and_do_not_commit_memory
  - tests/cli_external_full_pull.rs::a_named_cluster_snapshot_uses_designer_and_leaves_published_memory_alone
  - tests/cli_external_full_pull.rs::snapshot_only_preview_without_stage_returns_a_read_only_plan
  - tests/cli_external_full_pull.rs::snapshots_refuse_ambiguous_targets_before_selecting_an_executor
  - tests/cli_external_full_pull.rs::ordinary_external_publication_records_the_generation_before_a_concurrent_change
  - tests/cli_external_full_pull.rs::the_terminal_frame_carries_the_code_of_a_base_of_another_copy
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

**Что изменила версия 6.** Режим `--snapshot-only` разрешает ответ `rejected`
на `ready` и оставляет стадию вызывающему, с `publication: not_published` и
`memory: unchanged` в итоговом кадре. Опциональный `--prepare-merge-baseline`
добавляет к `ready` поле `merge_baseline`, если исполнитель подтверждает
одно и то же поколение рабочей конфигурации до и после выгрузки. Поле содержит
`id` подготовленной памяти, `generation`, `provider` и `base_generation`
из сравнимой текущей записи, либо `null`, если такой записи нет. Номер кадров
остаётся 1. Поле добавляется только в явном режиме запроса памяти для
объявленной файловой ИБ. Файловый снимок требует `ibcmd`. Если он недоступен,
вызов завершается до запуска платформы. Настройка `providers.pull` действует
на обычный `pull`. Прямой кластерный снимок даёт дерево для просмотра
без `merge_baseline`. `--snapshot-only --dry-run` отвечает обычным
конвертом с абсолютным `data.target_path`, без JSONL-кадров и запуска
платформы. Неоднозначный адрес и лишние сырые ключи подключения дают отказ
до выбора исполнителя. Ответ `published` в режиме снимка завершает вызов с
`invalid_argument`, `publication: unknown` и `memory: unchanged`.

**Что изменила версия 5.** Терминальный кадр использует общий конверт версии 7, включая запись сеанса из `error.sessions.list`. Кадры протокола сохранили формат.

**Что изменила версия 4.** Конверт в `result` — `CTR.WIRE.COMMAND-ENVELOPE` версии 6: у него
необязательное поле `warning_details` с предупреждениями, у которых есть машинный код. На базе
другой живой рабочей копии итоговый кадр несёт там `infobase_of_another_copy`. Кадры протокола
не изменились. Отказ конверта знает необязательное поле `layer` у кода `environment_unavailable`.

**Что изменила версия 3.** Перечень значений `mode_reason` тот же, что у
`CTR.WIRE.PULL-DATA` версии 9: добавилось `platform_rewrote_all`. Внешний полный pull —
полная выгрузка без обновления, перечня по факту не делает и этой причины сам не называет; значение появилось в схеме, потому что перечень
у форм общий. Код отказа `no_memory` в этой версии называется `untracked_base`. Конверт в
`result` знает и код `platform_release_missing` с полем отказа `platform_release`, и коды
`infobase_sessions` и `infobase_designer_busy` с полем отказа `sessions`
(`CTR.WIRE.COMMAND-ENVELOPE` версии 5).

**Что изменила версия 2.** Квитанция выбора несёт необязательное `reason` формы
`CTR.WIRE.PULL-DATA` версии 8: полная выгрузка файловой базы без ключа `providers.pull` называет
`whole`.

## Пример

```json
{"version":1,"nonce":"fb9824d8-6c77-4219-86b6-54f935a83c8e","phase":"ready","decision":"published"}
```
