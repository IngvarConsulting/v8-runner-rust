# Deep Dive

Этот документ описывает execution semantics и operational nuances `v8-runner` без дублирования
полного каталога команд. За точным пользовательским surface обращайтесь к
[CAPABILITIES.md](CAPABILITIES.md), за YAML-контрактом к [CONFIGURATION.md](CONFIGURATION.md).

## Навигация

- [Модель выполнения](#модель-выполнения)
- [source-set и change detection](#source-set-и-change-detection)
- [Пайплайн push](#пайплайн-push)
- [Проверка и тесты](#проверка-и-тесты)
- [Файловые сценарии и публикация](#файловые-сценарии-и-публикация)
- [Shared EDT](#shared-edt)
- [workPath, lock и interruption policy](#workpath-lock-и-interruption-policy)
- [MCP runtime semantics](#mcp-runtime-semantics)

## Модель выполнения

`v8-runner` разделяет public surface и execution model:

- CLI и MCP являются разными публичными поверхностями.
- Use case слой остаётся transport-neutral orchestration boundary.
- Platform DSL и process execution остаются ниже use case слоя.
- Text output и machine-readable envelope проектируются отдельно от доменного результата.

Это позволяет держать один orchestration model для CLI и MCP, не смешивая `clap`, `Presenter` и
MCP DTO в одном слое.

## `source-set` и change detection

`source-set` — минимальная единица оркестрации.

- Для `format=DESIGNER` используется один runtime context `designer-<sourceSetName>`.
- Для `format=EDT` используются два context-а:
  - `edt-<sourceSetName>` для решения, нужен ли export;
  - `designer-<sourceSetName>` для решения, что именно загружать в ИБ.
- Хеши конфигураций и расширений лежат в `workPath/infobases/<база>/hashes/<набор>.redb`,
  хеши исходников расширений-инструментов — в `hashes/tools/<имя>.redb` той же базы.
  `<база>` — имя объявленной базы; база, названная строкой соединения, помнится под ключом
  `@<хеш>` из нормализованного адреса без учётных данных.
  Адрес базы, исходный каталог и назначение набора проверяются вместе со снимком.
  Чужая память останавливает обычный `push` с диагностикой; полный `pull` или `push --full`
  создаёт новую.
- Кеш экспорта EDT и внешних артефактов остаётся общим в `workPath/hash-storages/`.
  Журнал поколений агента — `workPath/infobases/<база>/generation.json`, запись на набор с
  привязкой памяти набора: запись другой пары не даёт пропустить выгрузку.
- Копия файла версий набора лежит в `workPath/infobases/<база>/dump-info/<набор>/`:
  подменённый `ConfigDumpInfo.xml` в каталоге набора перед выгрузкой по изменившемуся, перед
  выборкой `ibcmd` (`--object`, идёт как `--sync`) и перед
  загрузкой Конфигуратором или агентом заменяется ею, после удачной команды она перенимает
  файл платформы. Исключения: выборочная выгрузка Конфигуратора, `push` через `ibcmd` и `push`
  через агента, получившего копию каталога (SFTP, общий каталог без ссылки), копию не меняют — файл в каталоге они не переписывают или их запись
  остаётся на стороне агента; сбой копию тоже не меняет.
- Generated Designer output для EDT flow живёт под памятью базы
  `workPath/infobases/<база>/designer/<sourceSetName>`; внешние обработки и отчёты и база с
  нераспознанным адресом — под `workPath/designer/<sourceSetName>`.
- Файл версий и версия формата в нём читаются до запуска платформы. Нет файла или в его корне
  нет версии формата — выгрузка по изменившемуся становится полной поверх каталога, а снимок
  EDT заменяется целиком; ответ и превью называют `FULL` и причину. Полная выгрузка поверх
  каталога спрашивает сторожа замены: незакоммиченное в каталоге набора — отказ `refusing to
  overwrite` до запуска платформы. Как `ibcmd config export` без `--sync` ведёт себя в
  непустом каталоге, не замерено. Сверки по версии формата пока нет: таблица «платформа →
  версия формата» держит только замеренные строки, а замера нет —
  [#403](https://github.com/IngvarConsulting/v8-runner-rust/issues/403).
- Прежняя общая память (`hash-storages/designer-*.redb`, `hash-storages/tool-*.redb`,
  `designer/<набор>` формата EDT, `agent/generation/`) не переносится и не удаляется: первый
  `push` после обновления грузит всё дерево.

Change detection выполняется on-demand во время build/export/load decision и не требует
background watcher. `push <SET>` ограничивает анализ, export/load decision и
runtime snapshot commit только указанным source-set.

## Пайплайн `push`

Для `DESIGNER`:

1. Анализ изменений по выбранным `source-set`.
2. Выбор partial/full path по изменённым файлам. Сегодня удаление, правка `Configuration.xml`,
   изменённый каталог и список частичной загрузки больше 20 файлов (с XML-описаниями,
   добавленными к изменённым модулям) дают full; порог не настраивается
   (ключ `push.partialLoadThreshold` отвергается), а эти переходы снимаются —
   [#379](https://github.com/IngvarConsulting/v8-runner-rust/issues/379).
3. Загрузка через выбранный backend.
4. Commit runtime snapshot только после успешного шага.

Для `EDT`:

1. Анализ выбранных EDT source-set.
2. Export затронутых EDT source-set в generated Designer representation.
3. Повторный анализ generated Designer files.
4. Load/apply generated files через `DESIGNER` или `IBCMD`.

Пайплайн намеренно не является атомарным across many `source-set`: поздний failure не откатывает
уже успешные ранние шаги.

## Проверка и тесты

`test` и `check` проектируются как часть того же локального цикла, а не как отдельная
эксплуатационная подсистема.

- `test` сначала делает `push`, затем запускает YaXUnit или Vanessa Automation; с `--no-push`
  сборки нет, и тесты идут в уже подготовленной базе.
- `check designer-*` работает только для `DESIGNER` source format.
- `check edt` использует EDT `validate` и привязан к `format=EDT`.
- Таймауты и interruption metadata должны проходить через общий command-level contract, а не
  жить как ad hoc special case конкретной команды.

## Файловые сценарии и публикация

Важно различать три разных класса файловых операций:

### `pull`

Это reverse sync из ИБ обратно в файловые исходники.

- Для `DESIGNER` может быть full, incremental или partial.
- Для `IBCMD` object-scoped partial деградирует в incremental.
- Полный `pull` в формате `DESIGNER` сначала считает хеши staging, затем публикует
  дерево и записывает эти хеши. Это общий путь Конфигуратора, `ibcmd` и агента.
  Отказ до публикации оставляет память прежней; после публикации ошибки хеширования
  или записи памяти становятся предупреждением с предложением повторить полный `pull`.
  Правка опубликованного дерева остаётся изменением.
- Цель полной выгрузки не может содержать `workPath`: замена удалила бы состояние команды.
- Для `format=EDT` использует internal Designer snapshot, затем EDT import.

### `convert`

Это repo-aware файловая конвертация текущих project files между `DESIGNER` и `EDT`.

- Не использует ИБ.
- Не является alias для `pull`.
- Работает только в модели `v8project.yaml` + `source-set`.

### `upload`, `make`, `artifacts`

Это materialization сценарии поверх готовых артефактов или publish targets.

- `upload` работает с готовыми `.cf` / `.cfe`.
- `make` / `artifacts` публикуют final `.cf`, `.cfe`, `.epf`, `.erf`.
- Full replacement target publication идёт через staged publication model.

## Shared EDT

`tools.edt_cli.interactive_mode` включает shared interactive EDT execution model.

- `false` означает one-shot `1cedtcli`.
- `true` означает shared actor/manager и одну interactive session для поддержанных EDT-сценариев.
- Для CLI shared EDT стартует лениво при первом EDT-вызове.
- `tools.edt_cli.auto-start` относится только к long-lived host process, сейчас это MCP server.

Shared EDT нужен не ради отдельного public режима, а ради повторного использования одного
execution model для CLI и MCP.

Проверку проекта EDT — `check` для формата EDT и `check_syntax_edt` — выполняет один
исполнитель для CLI и MCP. В shared-режиме у команды сессии нет кода выхода, и исход
читается по её выводу и журналу `--file`: stderr даёт `tool_failed`, замечания журнала —
`issues_found`, stdout без замечаний — `tool_failed`, а `exit_code` в ответе — `101` или `-1`. Предел
`command_timeout_ms` действует на каждый проект отдельно.

## `workPath`, lock и interruption policy

`workPath` является корнем runtime state.

- Логи, temp files, generated outputs и persisted snapshots не должны расползаться по каталогу primary config.
- Public CLI/MCP команды, работающие с runtime state под `workPath`, должны брать workspace lock.
- Workspace lock сериализует доступ к конкретному runtime root, но не заменяет admission limits и
  не делает multi-step orchestration fully atomic.
- Файловую базу команда держит своим замком рядом с каталогом базы, взятым после workspace lock:
  две рабочие копии с разными `workPath` одновременно с одной базой не работают, вторая получает
  `infobase_busy`. Метка владельца базы между командами — [#327](https://github.com/IngvarConsulting/v8-runner-rust/issues/327).

Interruption policy:

- timeout/cancellation являются общим CLI/MCP contract;
- terminal cancellation и deferred interruption должны различаться;
- отмена — род `interruption` у любой команды, и решает это сама ошибка, а не сигнал: отказ,
  пришедший при ожидающей отмене, остаётся отказом;
- в формах с итогом исполнения `test`, `upload`, `make`, `download`, `infobase dump` и
  `infobase restore` остановку отменой пишет один владелец (`record_cancellation`): статус
  `cancelled`, ошибка `cancelled` и запись о прерывании с одним текстом;
- critical publish/apply phases не hard-kill by default; запись в базу, и `/RestoreIB` тоже,
  дорабатывает до конца.

## MCP runtime semantics

MCP deliberately narrower than CLI.

- Опубликованы только 8 tool-операций.
- `CallToolResult` / `isError` остаются MCP-native protocol behavior.
- Business failure payload uses the shared command envelope.
- HTTP session capacity и execution admission являются разными guardrails.
- Shared EDT under MCP reuses the same execution model instead of inventing a separate MCP-only
  runtime path.
