# Возможности

Публичный каталог команд и текущих поддержанных сценариев `v8-runner`.

Документ описывает только текущий пользовательский контракт. Если он расходится с кодом или live
CLI help, доверяйте текущему коду и затем синхронизируйте docs.

## Навигация

- [Матрица поддержки](#матрица-поддержки)
- [Глобальные CLI-опции](#глобальные-cli-опции)
- [Настройка проекта](#настройка-проекта)
- [Проверка и валидация](#проверка-и-валидация)
- [Файлы и артефакты](#файлы-и-артефакты)
- [Прямой запуск и MCP](#прямой-запуск-и-mcp)
- [workPath и артефакты выполнения](#workpath-и-артефакты-выполнения)
- [Пока не поддерживается](#пока-не-поддерживается)

## Матрица поддержки

| Сценарий | Поддерживаемые комбинации | Примечания |
| --- | --- | --- |
| `version` | Работает без существующего конфига | Печатает имя приложения и версию; с `--json-message` возвращает JSON envelope |
| `clone` | Работает без существующего конфига | Создаёт проект из существующей ИБ: config, local overlay, `.gitignore`, `src/configuration` |
| `init` | Работает без существующего конфига | Создаёт `v8project.yaml`, sibling `v8project.local.yaml`, `.gitignore` entry, autodetect-ит supported `source-set` и aggregate external roots |
| `tools download <tool>` | CLI-only загрузка latest releases | Загружает выбранный YAxUnit, Vanessa Automation single или onec-client-mcp-devkit; обновляет local overlay для Vanessa/client MCP и при `yaxunit --sources` добавляет YAxUnit как `source-set` `tests` |
| `infobase create` | провайдер `designer` (умолчание) или `ibcmd` | Конфигуратор создаёт файловую ИБ, серверную оставляет ручной предпосылкой; `providers.infobase.create: ibcmd` создаёт файловую или серверную через `ibcmd infobase create` (серверной нужна `infobase.dbms`); при `format=EDT` дополнительно импортирует EDT workspace |
| `extensions` | `format=DESIGNER` или `format=EDT`; провайдер `ibcmd`, `agent` только по `providers.extensions: agent` | Обновляет свойства extension `source-set` или установленного расширения, названного платформенным именем (`--installed-name`); `list`/`info`/`create`/`delete`/`activate` — состав расширений ИБ; у `agent` всё это группа `config extensions` одной сессией на команду, состав читается из структурного ответа `properties get`, синоним при `create` уходит в форме `NStr()` |
| `push` | цепочка `designer` → `ibcmd`, любой `format`; `agent` только по `providers.push: agent` при `format=DESIGNER`; у автономного сервера (`infobase.standalone`) — только `agent` через SSH-шлюз сервера, платформа на машине раннера не нужна | Incremental/full загрузка в ИБ; при `format=EDT` сначала экспортирует изменённые EDT `source-set`; у `agent` загрузка и `update-db-cfg` — одна сессия на команду, исходники выставляются агенту ссылкой в `AgentBaseDir`, после загрузки записывается поколение конфигурации |
| `test` | Та же матрица, что и у `push` | По умолчанию запускает `push` |
| `test --no-push` | Подготовленная file/server ИБ; source-set и build tooling не требуются | Запускает выбранный test engine без `push` |
| `pull` | цепочка `designer` → `ibcmd`, любой `format`; `agent` только по `providers.pull: agent` при `format=DESIGNER` | Полная, инкрементальная или object-scoped partial выгрузка; у `ibcmd` `partial` деградирует в incremental с warning; у `agent` `incremental` и `partial` обновляют цель на месте через ссылку в `AgentBaseDir`, `full` публикуется через staging; перед `full` и `incremental` агента спрашивают поколение конфигурации, и равное записанному после последней сборки или выгрузки через агента означает «выгружать нечего»; при `format=EDT` — reverse sync через internal Designer snapshot и EDT import; перед заменой каталога цели раннер спрашивает git, что в нём не восстановить, и найдя незафиксированное, файл вне учёта или в игноре, отказывает с выходом 2 и называет потери, а `--force` уничтожает их без копии; там, где git не отвечает, поведение прежнее и защиты нет |
| `download` | цепочка `designer` → `ibcmd`; `agent` только по `providers.download: agent` | Выгружает working/database configuration в `.cf` или named extension в `.cfe`; раннер берёт первого готового до spawn, квитанция называет пропущенных; у `agent` только `working` (`config dump-cfg`, команды для конфигурации базы данных у агента нет — `database` отказывает до сессии), файл пишется в каталог агента и переносится в staging |
| `infobase dump` | провайдер `designer`; `ibcmd` только по `providers.infobase.dump`; `agent` только по `providers.infobase.dump: agent` | Выгружает полную ИБ в переносимый `.dt`; это не backup; `ibcmd` остаётся experimental до exclusive-access preflight; у `agent` — `infobase-tools dump-ib` в каталог агента и перенос в staging |
| `convert` | CLI-only repo-aware конвертация текущих `source-set` | Строки в матрице провайдеров не имеет и не требует ИБ |
| `upload` | `format=DESIGNER`, провайдер только `designer` | Загрузка `.cf` / `.cfe` артефактов в ИБ |
| `make` / `artifacts` | `format=DESIGNER`, провайдер `designer`; `agent` только по `providers.make: agent` | Экспорт `.cf` / `.cfe` и публикация `.epf` / `.erf`; у `agent` `.cf`/`.cfe` — `config dump-cfg` в каталог агента, `.epf`/`.erf` — исходники копируются в каталог агента (файловые параметры через ссылку агент не разрешает), сборка `load-external-…-from-files` и обратная выгрузка для сверки вида и имени, как у Конфигуратора |
| `check` | `format=DESIGNER` или `format=EDT` | Designer checks для `DESIGNER`, EDT `validate` для `EDT` |
| `infobase restore` | провайдер `designer`; `ibcmd` только по `providers.infobase.restore`; `agent` только по `providers.infobase.restore: agent`; у автономного сервера (`infobase.standalone`) строки нет — снимок снимают средствами сервера | Загрузка полной ИБ из DT; обязателен `--create` или `--replace`; у `agent` DT подкладывается в каталог агента жёсткой ссылкой или копией, `infobase-tools restore-ib`, после чего агент сам завершает сеанс и рвёт соединение — это не ошибка |
| `launch` | Не зависит от `format` | Прямой запуск 1C utility по позиционному mode; `launch web` открывает `infobase.web.url` в браузере, а `launch thin --via web` — тонким клиентом по тому же адресу |
| `publish` | Файловая и кластерная база, провайдер только `webinst` | Публикует базу на веб-сервере из `infobase.web`; `--delete` снимает публикацию; `--dry-run` показывает команду `webinst` со всеми параметрами и замаскированным паролем в `-connstr` |
| MCP | `stdio` и `streamable HTTP` | Публикует 8 инструментов, уже более узкая поверхность, чем CLI |

### Экспериментальные исполнители

Строки матрицы с пометкой «экспериментально» в цепочку умолчаний не входят: раннер сам такого
исполнителя не выберет. Включается он ключом `providers.<операция>: <исполнитель>` в
`v8project.yaml` или в личном `v8project.local.yaml`, выключается удалением ключа —
других выключателей (флага CLI, переменной окружения, состояния на диске) нет. Что
включилось, видно по квитанции ответа: `provider.selected` и `provider.origin.kind: override`.
Экспериментальный исполнитель не откатывается на следующего по цепочке: если он не готов,
команда отказывает. Сейчас экспериментальны `agent` у `push`, `pull`, `make`, `extensions`
и `download` на файловой базе и кластере, а также `ibcmd` у
`infobase dump` и `infobase restore`; у автономного сервера `agent` — единственный
исполнитель, ключ ему не нужен и не разрешён. Подробнее — раздел «Эксперименты» на
[сайте](https://ingvarconsulting.github.io/v8-runner-rust/architecture.html).

## Превью у глаголов, работающих с платформой

`--dry-run` — глобальный ключ: он значит одно и то же перед командой и после неё.
Исполняют его `push`, `upload`, `pull`, `download`, `make`/`artifacts`, `convert`,
`launch`, `publish`, `check`, `extensions` со всеми подкомандами и
`infobase create|dump|restore`.
У `version`, `clone`, `init`, `tools download`, `test` и `mcp serve` превью нет:
ключ там отвергается с названной причиной, а не исполняется молча.

**Квитанция об исполнителе одна у всех.** Каждая операция, у которой есть строка в
матрице провайдеров, кладёт в ответ `provider`: `selected` — кто выбран, `origin` —
умолчание матрицы или ключ `providers.*` с именем файла, `skipped[]` — кого пропустили
и почему. Раннер берёт первого готового из цепочки умолчаний; переопределение не
откатывается: `selected: null` и список пропущенных с причиной.

**Форм превью две, и это не недосмотр.** У `infobase`-экспорта и `restore` превью
отвечает экспортным конвертом (`mode=preview`, `plan.provider`); у остальных превью
отвечает парой `provider_dispatched` + предмет глагола. Их превью отвечает парой
**`provider_dispatched` + предмет глагола**:

| Глагол | Что называет превью |
|---|---|
| `launch` | `plan.program` и составленный `plan.args` с замаскированными credential |
| `convert` | `outputs` — что и куда было бы сконвертировано |
| `infobase create` | по шагу `status: planned` с тем, что было бы создано и чем |
| `push` | по набору исходников планируемый `mode` и причину |
| `upload` | артефакт, режим, расширение; `compatibility_state: not_probed` |
| `pull` | набор, режим, целевой путь |
| `artifacts` | вид артефакта и выход, `published: false` |
| `extensions` | целевые имена, отключаемые свойства безопасности, ИБ, учётку и путь `ibcmd` |
| `check` | `status: planned`, команду платформы с режимами и найденную утилиту |

- **Закрытый признак есть у обеих форм, но он разный.** У экспортной это `mode`
  (`preview` против `apply`), и `provider_dispatched` там появляется только в
  превью. У формы `launch` это само `provider_dispatched`, и оно присутствует в
  обоих режимах. Читать отсутствие поля как «ничего не запускали» не приходится ни
  там, ни там — но поле для этого у каждой формы своё.
- Превью возвращается **после** поиска утилиты: отсутствующая платформа
  отказывает до одобрения плана, а не после.
- **Превью не берёт workspace lock и не ждёт его.** Значит «покажи план» не
  упирается в занятое пространство и два одновременных превью не выстраиваются в
  очередь. `--clean-before-execution` с превью отклоняется, а не пропускается:
  чистка меняет `workPath`.
- Превью не создаёт ни целевых каталогов, ни артефактов, ни staging, ни
  EDT-рабочего пространства, ни состояния обнаружения изменений. В журнале
  действий (`workPath/logs`) строка о вызове остаётся — как у любого запуска;
  превью не прячется, оно не меняет предмет.

**Названные пределы, а не умолчания:**

- `upload` возвращается до зонда совместимости, потому что зонд сам запускает
  конфигуратор. Поэтому состояние совместимости — `not_probed`, и это отдельное
  значение от `unknown`: «не спрашивали» и «спросили и не получили ответа» —
  разные факты для того, кто решает, применять ли.
- `infobase create` для серверной ИБ не различает «создана» и «уже была»: это различие даёт
  сама `ibcmd infobase create`, то есть действие. Превью называет цель и утилиту
  и на этом останавливается.
- `push` в формате EDT планирует шаг экспорта целиком: выгрузка в файлы
  конфигуратора и последующая загрузка в базу не разделяются, потому что вторая
  зависит от результата первой. Два исключения названы: у внешнего набора
  загрузки за экспортом нет вовсе, а когда экспорт пропущен за отсутствием
  изменений, загрузка планируется своим шагом.

## Рода и коды отказа

`error.kind` и `error.code` в конверте — закрытые перечисления, а не свободные строки:
новый род приходит вместе с версией конверта, а не молча. Схема порождается из типов
(`UPDATE_ENVELOPE_SCHEMA=1 cargo test generated_envelope_schema_is_current`), руками её не
правят.

| Род | Коды | Код выхода |
| --- | --- | --- |
| `capability` | `capability_unavailable`, `subject` (предмет не тот, навсегда), `target` (не для этой цели), `soon` (пока не умеет) | 2 |
| `environment` | `environment_unavailable` | 2 |
| `validation` | `invalid_argument`, `unsupported_value` (только MCP) | 2 |
| `workspace` | `workspace_busy` | 3 |
| `runtime` | `runtime_failure` | 3 |
| `platform` | `platform_failure` | 4 |
| `invalid_output` | `invalid_output` | 4 |
| `interruption` | `cancelled`, `timed_out` | 4 |
| `non_fast_forward`, `no_memory` | одноимённые коды | — |

Последняя строка заведена без производителя: рода названы, чтобы набор не рос молча, но
выдавать их будут сравнение поколений и память по базе. Тогда же у `non_fast_forward`
заполнятся `base_generation` и `local_generation`. Код `subject` таблица называет, но ни
один отказ пока им не отвечает.

У MCP словарь уже: рода там сводятся к `validation`, `runtime` и `platform`, поэтому кода
возможности в ответе инструмента не бывает. Поле `next` едет обоими транспортами.

Отказ, у которого есть выход, называет его полем `error.next`: `command`, при нужде
`source_set` и ключи. Проза сообщения при этом не сокращается — она остаётся человеку.
Код шага исполнителя внутри `data.execution.errors[]` — другой словарь: совпадение имён
не делает их одним полем.

## Глобальные CLI-опции

| Опция | Значение |
| --- | --- |
| `--version` | Печатает версию приложения и завершает выполнение |
| `--config <CONFIG>` | Путь к существующему `v8project.yaml`; по умолчанию `./v8project.yaml` |
| `--json-message` | Structured JSON envelope вместо text output |
| `--log-level <LOG_LEVEL>` | `error`, `warn`, `info`, `debug`, `trace` |
| `--clean-before-execution` | Очистить лог-файлы перед запуском |
| `--no-color` | Отключить ANSI-цвета |
| `--workdir <WORKDIR>` | Переопределить `workPath` из конфига |
| `--infobase <NAME\|CONNECTION>` | База: имя из карты `infobases` местного слоя или строка соединения целиком; без ключа берётся `origin`. У `init` этот ключ базу объявляет, а не выбирает |
| `--dry-run` | Превью: показать план, ничего не запуская. Команда без превью ключ отвергает |

Если рядом с primary config лежит `v8project.local.yaml`, он применяется автоматически до CLI
overrides. Сам local overlay нельзя передавать как `--config`.

Принципы вывода:

- Без `--json-message` CLI держит clean success path кратким.
- Live progress в text output использует human-readable строки; для long-running stages время
  старта может выводиться как локальный префикс `HH:MM:SS`, без structured ключей вроде
  `started_at`.
- Важные warnings, degraded behavior, diagnostics и created artifacts должны быть видимы и в text,
  и в JSON.
- `--json-message` остаётся machine-readable contract для автоматизации.
- MCP `structured_content` использует тот же envelope core: `ok`, `command`, `duration_ms`,
  `data`, `warnings`, `steps`, optional `error`.

## Настройка проекта

### `version`

```bash
v8-runner version
v8-runner --version
```

- Не требует `v8project.yaml`.
- В text mode печатает `v8-runner <version>`.
- С `--json-message` команда `version` возвращает envelope с `data.name` и `data.version`.

### `init`

```bash
v8-runner init [--force] [--output <FILE>] [--connection <CONNECTION>] [--format <auto|designer|edt>]
```

- Не требует существующего `v8project.yaml`.
- Пишет результат в текущий каталог или в `--output`.
- Рядом с primary config создает/обновляет пустой `v8project.local.yaml` со schema modeline и
  добавляет `v8project.local.yaml` в `.gitignore`, если подходящий pattern еще не указан.
- Не использует глобальный `--config` как shortcut output path.
- Ищет supported `DESIGNER` / `EDT` `source-set` по marker files и их содержимому.
- Для external roots создаёт aggregate `source-set` только при однородной классификации каталога.
- Не пишет synthetic `CONFIGURATION`: отсутствие конфигурационного source-set это validation error.

### `clone`

```bash
v8-runner clone --connection <CONNECTION> --platform-version <VERSION> [--project-dir <DIR>] [--source-dir <DIR>] [--user <USER>] [--password <PASSWORD>] [--platform-path <PATH>] [--force]
```

- Работает до загрузки `v8project.yaml` и предназначен для пустого project directory.
- Создаёт `v8project.yaml`, schema-modelined `v8project.local.yaml`, `.gitignore` entry и
  `source-set main` типа `CONFIGURATION`.
- Выгружает основную конфигурацию из указанной ИБ в `src/configuration` через Designer full dump.
- `--connection` не должен содержать embedded credentials; используйте `--user` и `--password`.
  Эти значения пишутся только в `v8project.local.yaml`.
- Не обнаруживает и не выгружает расширения автоматически.

### `infobase create`

```bash
v8-runner infobase create [--dry-run]
```

- Всегда разделяет шаг подготовки ИБ и шаг EDT workspace.
- Для file connection Конфигуратор (умолчание) использует `1cv8 CREATEINFOBASE`.
- При `providers.infobase.create: ibcmd` использует `ibcmd infobase create`; server path добавляет
  `--create-database` и требует `infobase.dbms`.
- При `ibcmd` неудачное создание считается «база уже есть» только если сама база
  после этого читается: спрашивается `config generation-id`, и ноль она отвечает
  лишь когда база существует и эти учётные данные её читают. Формулировка отказа
  в решении не участвует (DEC.2026-09-12.TOOL-PROSE-NEVER-DECIDES), поэтому отказ авторизации и незаписываемый
  путь остаются ошибкой, а не «уже есть».
- Для `format=EDT` использует `workPath/edt-workspace` и импортирует `CONFIGURATION`, затем
  `EXTENSION`.
- Если настроен `tools.client_mcp.extension.source.format=EDT`, импортирует этот tool extension
  project в EDT workspace, не добавляя его в project `source-set`.

### `tools download`

```bash
v8-runner tools download yaxunit [--sources] [--force]
v8-runner tools download vanessa [--force]
v8-runner tools download client-mcp [--sources] [--force]
```

- CLI-only; не публикуется как MCP tool.
- Берёт latest release из GitHub для выбранного инструмента: `bia-technologies/yaxunit`,
  `Pr-Mex/vanessa-automation-single` или `1c-neurofish/onec-client-mcp-devkit`.
- `yaxunit --sources` распаковывает source subtree в `tests` и добавляет в primary
  `v8project.yaml` `source-set` с именем `tests`; без `--sources` скачивает `.cfe` в
  `build/tools`.
- `client-mcp --sources` распаковывает source subtree в
  `build/tools/onec-client-mcp-devkit/exts/client-mcp`; без `--sources` требует, чтобы
  сборку исполнял Конфигуратор, и скачивает `.cfe` в `build/tools`.
- `vanessa` всегда скачивает `build/tools/vanessa-automation-single.epf`.
- `v8project.local.yaml` обновляется только для команд, которым нужны machine-local пути:
  `vanessa` заполняет `tools.va.epf_path`, `client-mcp` заполняет
  `tools.client_mcp.extension`; повторный запуск переиспользует уже скачанные файлы, а
  `--force` перезаписывает только managed targets, созданные `tools download`.
- Managed target определяется sidecar marker-файлом `tools download`; если публикация файла или
  каталога не завершилась, новый marker очищается и target не считается управляемым.
- Каждый HTTP response body ограничен 512 MiB; превышение лимита возвращает ошибку до публикации
  target.

### `extensions`

```bash
v8-runner extensions [--name <SOURCE_SET>...] [--installed-name <PLATFORM_NAME>...] [--dry-run]
v8-runner extensions --name TESTS --installed-name YAXUNIT
```

- Отключает безопасный режим и защиту от опасных действий через IBCMD.
- `--name` выбирает только `source-set` с `type=EXTENSION`; неизвестное имя — ошибка.
- `--installed-name` передаёт платформенное имя установленного расширения без требования
  соответствующего `source-set`. Валидный конфиг проекта всё равно нужен.
- Без обоих селекторов обрабатывает все extension `source-set` из конфига. При наличии
  любого селектора обрабатывает только явно выбранные цели: сначала `--name`, затем
  `--installed-name`. Точные повторы выполняются один раз; регистр и пробелы сохраняются.
- Пробельное/пустое имя, управляющие символы и начальный `-` в `--installed-name`
  отклоняются до блокировки, очистки и вызова платформы. Селекторы нельзя смешивать с подкомандами.
- Возвращает пошаговый результат по каждому целевому расширению.
- `--dry-run` разрешает цели и находит `ibcmd`, возвращает планируемые `steps` с
  `provider_dispatched=false`, не запускает платформу и не трогает `workPath`.
  Наличие расширений в ИБ в превью не проверяется. С `--clean-before-execution` несовместим.
- Ошибка обновления, в том числе отсутствующего расширения, возвращает platform error
  и неуспешный шаг с целевым именем; следующие цели не выполняются.

#### Состав расширений информационной базы

```bash
v8-runner extensions list [--dry-run]
v8-runner extensions info --name <NAME> [--dry-run]
v8-runner extensions create --name <NAME> --name-prefix <PREFIX> [--synonym <NSTR>] [--purpose <customization|add-on|patch>] [--dry-run]
v8-runner extensions delete --name <NAME> [--dry-run]
v8-runner extensions activate --name <NAME> --active <yes|no> [--dry-run]
```

- `extensions` без подкоманды правит свойства безопасности выбранных расширений;
  подкоманды читают и меняют состав расширений, **установленных в информационной базе**.
  Их `--name` всегда означает платформенное имя.
- **Семейство IBCMD-only.** У Designer нет батч-ключа, который перечисляет
  установленные расширения, поэтому у `extensions` в матрице один исполнитель —
  `ibcmd`; при его отсутствии операция отказывает, а не уходит на Designer.
- `list` и `info` отдают по расширению: `name`, `version`, `active`, `purpose`,
  `safe_mode`, `security_profile_name`, `unsafe_action_protection`,
  `used_in_distributed_infobase`, `scope`, `hash_sum`. Пустое поле платформы —
  отсутствующее значение, а не пустая строка: в JSON его просто нет.
- **Предмет чтения — поле.** И превью, и ответ несут `requested`:
  `{"kind": "all"}` у `list`, `{"kind": "named", "name": …}` у `info`. Сверять
  превью со своим запросом нужно по нему, а не по строке `plan`: она для
  человека.
- **Префикса имён на чтении нет.** Платформа не сообщает `name-prefix` ни в
  `list`, ни в `info`; он живёт только в `Configuration.xml` самого расширения,
  то есть достаётся выгрузкой.
- **Порядок не обещается.** Платформа выдаёт записи не в порядке создания и
  порядок не документирует, поэтому и runner его не обещает.
- `info --name` проверяет, что платформа ответила про запрошенное расширение:
  ответ про другое — `invalid_output`, а не тихая подмена.
- Вывод платформы текстовый и разбирается runner-ом; запись без обязательного
  поля и `yes/no`-поле с иным значением отклоняются, а не получают умолчание.
- `activate` существует отдельно от подключения: платформа даёт активность
  самостоятельным ключом `--active`.
- Чтение и запись делят одну границу workspace lock: состав может измениться под
  чтением, и перечень, снятый поперёк установки, показал бы половинное состояние.
- **`--dry-run` есть и у читающих подкоманд.** «Просто прочитать» не бывает:
  поднимается `ibcmd`, открывается соединение, проходит аутентификация, в журнале
  остаётся след — значит чтение состава это действие, и превью ему нужно так же,
  как изменению. Превью называет цель, учётку и утилиту, а `extensions` в нём пуст,
  потому что у платформы ничего не спрашивали.
- Сырая строка соединения в превью не воспроизводится: она может нести `Pwd=`,
  поэтому называются только узнаваемые части цели и имя учётки.
- Требуемое право превью не называет: платформа его не сообщает, и выдумывать имя
  права оно не станет.

### `push`

```bash
v8-runner push [--source-set <NAME>] [--full] [--dry-run]
```

- Без `--source-set` обрабатывает все configured `source-set` в canonical order.
- С `--source-set` project stage анализирует и строит только указанный `source-set`; неизвестное
  имя отклоняется как validation error.
- Для `DESIGNER` выбирает incremental, partial или full path по изменённым файлам выбранного scope.
- Для `EDT` сначала анализирует и экспортирует выбранные EDT `source-set`, затем грузит generated
  Designer files выбранным backend.
- После успешного project stage, включая scoped `--source-set`, подготавливает
  `tools.client_mcp.extension`, если оно настроено: `source` загружается как extension из
  исходников, `.cfe` `artifact` загружается как extension с именем
  `tools.client_mcp.extension.name`. Под `--dry-run` подготовки не происходит: превью
  находит утилиту, называет режим, который был бы применён, и на этом останавливается.
- Для source-backed `tools.client_mcp.extension` использует отдельное состояние change detection
  под `workPath/hash-storages`: неизменённый source пропускает export/load, `--full`
  принудительно обновляет расширение.
- `tools.client_mcp.extension` не является project `source-set`; `--source-set` выбирает только
  project source-set.
- Не является атомарной multi-source-set операцией: ранние успешные шаги не откатываются, если
  поздний шаг падает.

## Проверка и валидация

### `test`

```bash
v8-runner test [--full] [--no-push] yaxunit all
v8-runner test [--full] [--no-push] yaxunit module <NAME>
v8-runner test [--no-push] va
v8-runner test [--no-push] va --feature login --filter-tag @smoke
```

- По умолчанию сначала запускает `push`. `--no-push` отмечает build-step как `skipped` и
  запускает тесты на подготовленной ИБ; для file connection до запуска платформы требуется
  `<infobase>/1Cv8.1CD`, для server connection доступность подтверждается запуском test engine.
- В `--no-push` source-set и build tooling не проходят filesystem/layout validation: исходники
  configuration могут отсутствовать. Валидация ИБ, платформы и настроек test engine сохраняется.
- `--no-push` является CLI-only контрактом; MCP `run_all_tests` сохраняет build-first поведение.
- `test yaxunit module <NAME>` требует непустое имя модуля.
- `test va` использует профиль из `tests.va.profile`; `--feature`, `--filter-tag`,
  `--ignore-tag` и `--scenario-filter` переопределяют соответствующие списки выбранного профиля
  только для текущего запуска.
- Для функциональных `.feature`-сценариев и приемки используйте Vanessa Automation: CLI
  `test va` или MCP `run_all_tests` с `runner=vanessa`, а не дефолтный YaXUnit-runner.
- `--full` включает полный вывод успешных кейсов и расширенные stack traces.
- `tests.*.timeouts.total_ms` остаётся активным пользовательским контрактом таймаутов.

### `check`

```bash
v8-runner check [MODE FLAGS] [--dry-run]
v8-runner check --project <PROJECT>... [--dry-run]
```

Команда одна, ветку выбирает `format` проекта.

`format=DESIGNER` — `/CheckConfig` Конфигуратора:

- Режимы платформы называются ключами команды: проверки конфигурации и области клиента.
- Ни один режим не назван — выполняется профиль по умолчанию: `-ThinClient`, `-Server`,
  `-UnreferenceProcedures`, `-HandlersExistence`, `-EmptyHandlers`, `-ExtendedModulesCheck`.
  Пустая `/CheckConfig` не проверяет ничего и отвечает «чисто», поэтому пустой она не
  вызывается.
- Назван хотя бы один режим — выполняются ровно названные.
- Поддерживает `--extension <EXTENSION>` или `--all-extensions`.
- `--project` здесь не исполняется и отвергается.

`format=EDT` — проверка проекта средствами EDT CLI:

- Исполнитель — EDT CLI, строки в матрице провайдеров нет; база не нужна.
- Повторяемый `--project`; без него берутся все EDT-проекты конфига.
- Режимы `/CheckConfig` здесь не исполняются и отвергаются.

Проект, у которого все наборы исходников внешние, получает отказ рода `capability` с кодом
`subject`: проверка внешних обработок и отчётов платформой не описана.

`--dry-run` доходит до поиска утилиты и возвращается раньше собственных записей команды:
каталог журналов платформы не создаётся, платформа не запускается. Сам вызов при этом
остаётся в журнале действий — превью не прячется (`INV.CLI.PREVIEW-LEAVES-A-LOG-ENTRY`). Ответ называет `status: planned`,
`provider_dispatched: false` и `exit_code: -1` — кода выхода не наблюдалось. Поле `message`
говорит, что было бы выполнено: команда платформы с режимами и найденная утилита. Обе
ветки останавливаются одинаково.

Прежние имена `check designer-config`, `check designer-modules` и `check edt` приняты один
цикл скрытыми синонимами. Отдельного пути `/CheckModules` не осталось: его режимы целиком
покрыты `/CheckConfig`, и ответ такого вызова называет `check_name: designer-config`.

## Файлы и артефакты

### `pull`

```bash
v8-runner pull --mode <full|incremental|partial> [--source-set <NAME>] [--extension <EXTENSION>] [--object <TYPE:NAME>...] [--dry-run] [--force]
```

- `partial` требует хотя бы один `--object`.
- Перед заменой каталога исходников команда спрашивает git, что нельзя вернуть:
  неотслеживаемые и игнорируемые файлы, правки рабочего дерева, неразрешённые маркеры
  слияния. Найдя такое, она отказывает с кодом выхода 2 и называет файлы;
  `--force` заменяет каталог всё равно. Там, где git не отвечает,
  поведение прежнее и защиты нет.
- Канонический ввод селектора — `TYPE:NAME` (например, `Catalog:Items`); для
  совместимости принимается и `TYPE.NAME`. Переданный селектор сохраняется в JSON как
  `data.selectors[*].requested`, а в списке Designer и как
  `data.selectors[*].normalized` используется нормализованный `TYPE.NAME`.
- До запуска платформы CLI валидирует синтаксис селектора: непустые `TYPE` и `NAME`,
  ровно один разделитель `:` или `.`, без управляющих символов. У Конфигуратора
  существование metadata root type проверяет сам Designer; `ibcmd` не использует object list,
  потому что деградирует в incremental.
- Конфигуратор поддерживает true object-scoped partial.
- `ibcmd` не умеет object-scoped partial; запрос деградирует в incremental с warning.
- `format=EDT` использует internal Designer snapshot под `workPath/designer/<sourceSetName>`,
  затем импортирует его в EDT target и публикует результат атомарной заменой target каталога.

### `convert`

```bash
v8-runner convert [--source-set <NAME>] [--output <DIR>] [--dry-run] [--force]
```

- CLI-only; не публикуется как MCP tool.
- Перед заменой целевого каталога команда спрашивает git, что нельзя вернуть:
  неотслеживаемые и игнорируемые файлы, правки рабочего дерева, неразрешённые маркеры
  слияния. Найдя такое, она отказывает с кодом выхода 2 и называет файлы;
  `--force` заменяет каталог всё равно. Там, где git не отвечает,
  поведение прежнее и защиты нет.
- Работает от текущего `v8project.yaml`, а не по arbitrary source/target paths.
- Направление определяется только из `format`.
- Без `--output` публикует результат под `workPath/convert/out/<sourceSetName>/<designer|edt>/`.
- `--output` задаёт только target root и зеркалит `source-set.path` относительно каталога primary config.
- Публикация остаётся staged full replacement с overlap guardrails.

### `download`

```bash
v8-runner download --state <working|database> --output <FILE.cf> [--dry-run]
v8-runner download --state <working|database> --extension <NAME> --output <FILE.cfe> [--dry-run]
```

- Сохраняет состояние конфигурации из ИБ, а не собирает пакет из project sources.
- Без `--extension` экспортирует main configuration и требует `.cf`; с extension требует `.cfe`.
- Умолчание — цепочка `designer` → `ibcmd`: runner берёт первого готового до spawn и кладёт
  в квитанцию `provider`, кого пропустил и почему. `providers.download`
  назначает одного исполнителя без отката.
- Переключение допустимо только во время pure preflight; после первого spawn provider не меняется.
- Публикация идёт через sibling staging и target-specific lock; `published=true` означает, что
  финальный файл уже заменён атомарно.
- Target lock сериализует cooperating запуски runner. Параллельный внешний writer обязан
  использовать тот же lock или быть остановлен: path revalidation не является filesystem CAS.
- Ожидание чужого target lock ограничено пятью минутами. Это предел шага, а не срок команды
  (`DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE`): цель держит другой прогон, и дальнейшее
  ожидание ничего не изменит. По истечении окна отказ приходит как `workspace_busy`
  (`error.kind: workspace`, `execution.status: failed`, код возврата CLI `3`), а не как
  `timed_out` — раньше это окно задавал общий срок и отказ назывался таймаутом.
- После аварийного завершения owner lock может остаться на диске. Для совместимости с уже
  опубликованными версиями runner такой lock обрабатывается fail-closed: удалять его вручную можно
  только при остановленных старых и новых процессах runner.
- `execution.status` использует общий terminal vocabulary runner; он не заменяет `published`,
  который отдельно отвечает только за commit финального файла.
- `cancelled`, `timed_out` и `invalid_output` не сводятся к generic failure; ошибки считаются
  non-retryable по умолчанию, а provider не переключается после dispatch.
- `--dry-run` возвращает выбранный provider и compact plan, но не создаёт `workPath`, locks,
  staging/output и не запускает provider process. JSON явно сообщает
  `mode=preview`, `provider_dispatched=false` и `published=false`.

### `infobase dump`

```bash
v8-runner infobase dump --output <FILE.dt> [--dry-run]
```

- Сохраняет полную ИБ с данными в переносимый DT-файл. DT не является резервной копией.
- Умолчание — Designer. `ibcmd` для DT остаётся experimental: в цепочку умолчаний не входит и
  назначается только `providers.infobase.dump: ibcmd`.
- Если implemented provider есть, но binary/version/connection не готовы, возвращается
  `environment_unavailable`; `capability_unavailable` означает отсутствие implemented adapter.
- Для файловой ИБ readiness обоих process providers требует существующий файл
  `<infobase>/1Cv8.1CD`; один каталог или найденный бинарник не считаются готовой ИБ.
- Selection показывает каждого кандидата через независимые `implementation`, `readiness` и
  `evidence`; `argv_tested` не выдаётся за live proof.
- Безопасный IBCMD path требует проверки отсутствия активных сеансов.
- Обе операции требуют существующий `v8project.yaml`, но используют infobase-only validation:
  отсутствующий `source-set` равнозначен `source-set: []`, а сломанные project sources не блокируют чтение ИБ.
  Push/source/test/EDT/client-MCP настройки для этих команд не валидируются.

### `infobase restore`

```bash
v8-runner infobase restore --input <FILE.dt> --replace [--dry-run]
v8-runner infobase restore --input <FILE.dt> --create  [--dry-run]
```

- Парная операция к `infobase dump`: загружает полную ИБ вместе с данными из DT-файла.
- Ровно один режим цели обязателен. Ни один провайдер не спрашивает: Designer создаёт
  отсутствующую ИБ и перезаписывает существующую, IBCMD перезаписывает существующую.
  Поэтому шлюз ставит runner, и режим, не совпавший с наблюдаемой целью, — отказ
  `invalid_argument`, а не молчаливый переход к другому случаю:
  `--create` при существующей ИБ и `--replace` при отсутствующей отклоняются.
- Для файловой ИБ наличие цели определяется файлом `<infobase>/1Cv8.1CD`. Серверную ИБ без
  процесса наблюдать нельзя, поэтому там режим принимается на слово вызывающего, а последнее
  слово остаётся за платформой.
- Проверка цели идёт до выбора провайдера и повторяется под workspace lock: провайдер пишет
  прямо в ИБ, staging-шага здесь нет, и отменить загрузку нечем.
- `--input` проверяется до выбора провайдера: суффикс `.dt` и читаемый непустой файл.
- Implemented provider — Designer (`/RestoreIB`), подтверждён живым прогоном на 8.3.27.
  IBCMD `infobase restore` тоже запускается вживую, но остаётся `experimental`, пока нет
  проверки отсутствия активных сеансов — как и IBCMD DT export.
- `target_state` различает `created` и `replaced`. Если провайдер упал, `target_state` —
  `uncertain`: сколько данных он успел заменить, отсюда не видно.
- `restored=true` означает, что провайдер сообщил о завершённой загрузке.
- `--dry-run` проверяет запрос и цель, выбирает провайдера и возвращает `mode=preview`,
  `provider_dispatched=false`, `restored=false` и compact `plan` с `provider`, `input` и
  `target_mode`, но процесс не запускает.
- Принудительного завершения сеансов пока нет: ключ IBCMD `--force` не проброшен, у Designer
  такого ключа нет. Занятая ИБ отвечает ошибкой платформы.

### `upload`

```bash
v8-runner upload --path <FILE> [--mode <load|combine>] [--settings <FILE>] [--extension <NAME>] [--dry-run]
```

- Поддерживает `.cf` и `.cfe`.
- Работает только для `format=DESIGNER`; исполнитель — только Конфигуратор.
- `.cfe` требует `--extension`.
- `--mode combine` требует `--settings <FILE>`.
- Состояние совместимости имеет три публичных значения: `supported` — вопрос задан
  и доказан; `absent` — расширение доказано отсутствует в ИБ; `not_established` —
  вопрос задан и не доказан; `not_probed` — вопрос не задавался. Формулировки
  платформы на значение не влияют (DEC.2026-09-12.TOOL-PROSE-NEVER-DECIDES): у расширения состав читается
  структурно через `ibcmd config extension list`, у конфигурации сравнение считается
  состоявшимся только по нулевому коду выхода.
- Первая загрузка отсутствующего в ИБ расширения возвращает
  `compatibility_state=absent` и выполняется через `--mode load`; `combine` для
  такого расширения отклоняется с рекомендацией сначала выполнить `load`.
- `not_established` не разрешает изменяющую операцию ни в одном режиме: ни
  загрузку, ни слияние. Сюда попадают отказ авторизации, недоступная ИБ и
  нечитаемый состав расширений — всё, что платформа сообщает ненулевым кодом.
- `--mode combine` для конфигурации требует `--vendor-name <ИМЯ>`: без имени
  конфигурации поставщика платформа сравнение не выполняет, поэтому состояние
  остаётся `not_probed` и слияние отклоняется. `--mode load` имени не требует.
- `upload --mode update` не поддержан; используйте `load` или `combine`.

### `make` / `artifacts`

```bash
v8-runner make --output <TARGET> [--source-set <NAME>] [--extension <NAME>] [--dry-run]
v8-runner artifacts --output <TARGET> [--source-set <NAME>] [--extension <NAME>] [--dry-run]
```

- Это один use case с двумя CLI names.
- `.cf` используется для основной конфигурации.
- `.cfe` используется для extension export.
- Каталог output используется для external `.epf` / `.erf` publication.
- Исполнитель — только Конфигуратор.

## `publish`

```bash
v8-runner publish [--delete] [--dry-run]
```

- Параметры берутся из `infobase.web`, а не из флагов: публикация воспроизводится из файла.
- Составляет `webinst -publish|-delete -<server> -wsdir … -dir … [-connstr …] [-confpath …] [-osauth]`
  по грамматике платформы; предусловия называются до запуска — существующий `dir`,
  `conf` для apache2/apache22, `os-auth` только для iis.
- Публикация замещает `default.vrd` целиком, поэтому у команды есть превью; удаление —
  отдельный явный ключ `--delete`.
- Не входит ни в одну цепочку умолчаний: `push`, `test` и остальные публикацию не делают.
- Развилки нет: `providers.publish` отклоняется валидацией.

## Прямой запуск и MCP

### `launch`

```bash
v8-runner launch <designer|thin|thick|ordinary> [--via <web|connection>] [--dry-run] [FLAGS]
v8-runner launch web [--dry-run]
v8-runner launch mcp [va] [--mode <thin|thick|ordinary>] [--via <web|connection>] [--wait-ready] [FLAGS]
```

- Для обычного запуска (`designer`/`thin`/`thick`/`ordinary`) режим задаётся позиционным
  аргументом.
- `designer` использует `1cv8`.
- `thin` использует `1cv8c`.
- `--via` выбирает, каким из двух адресов цели открыть базу: `connection` —
  `infobase.connection`, `web` — `infobase.web.url` как ws-соединение. Умолчание задаёт вид
  цели: у автономного сервера — `web` (объявленная строка прямого шлюза раннером пока не
  используется, #205), у файловой и кластерной —
  `connection`. Ключ принимается только там, где клиент тонкий (`launch thin` и
  `launch mcp --mode thin`); у остальных режимов адрес один, и ключ отвергается. У
  автономного сервера отвергается и `--via connection`: объявленная строка прямого шлюза
  раннером пока не используется (#205).
- Ответ несёт `via` у каждого режима, а `url` — там, где адрес клиентский. Пароль из
  userinfo в показанном адресе замаскирован; в процесс уходит настоящий.
- `thick` и `ordinary` используют `1cv8`.
- `mcp` запускает клиентский MCP-сервер onec-client-mcp-devkit через `/C runMcp`.
- `launch mcp` по умолчанию использует `--mode thin` и `1cv8c`.
- `launch mcp --mode thick` использует `1cv8`; `launch mcp --mode ordinary` использует `1cv8`
  и добавляет `/RunModeOrdinaryApplication`.
- `launch mcp va` дополнительно запускает Vanessa Automation из `tools.va` через `/Execute <epf>`
  и передаёт `VAParams=<runtime params>` без `StartFeaturePlayer`.
- Для интерактивной отладки и написания функциональных `.feature`-сценариев используйте
  `launch mcp va --wait-ready`; голый `launch mcp` поднимает client MCP без Vanessa tools.
- Любой управляемый runner payload для ключа `/C` передаётся как значение отдельного
  аргумента `/C`: это касается `launch --c`, `launch mcp`, `test yaxunit` и `test va`.
  На уровне process argv это два элемента: `/C` и `<payload>`; shell-подобная запись
  `/C <payload>` в документации не означает один склеенный аргумент.
- Для `mcp` доступны typed flags `--mcp-config <FILE>` и `--mcp-port <PORT>`;
  итоговый payload: `/C runMcp[=<FILE>][;mcpPort=<PORT>]`.
- Если `--mcp-port` не указан, используется `tools.client_mcp.port` из `v8project.yaml`.
- `--wait-ready` ждёт `http://127.0.0.1:<port>/mcp`, выполняет MCP `initialize`,
  `notifications/initialized` и `tools/list`, а в JSON-результате возвращает `mcp_readiness`
  со списком tools. Для `launch mcp va --wait-ready` дополнительно проверяется наличие
  Vanessa tools: `load_features`, `open_feature_file`, `run_scenario`, `get_test_results`,
  `connect_test_client`.
- Timeout ожидания задаётся `tools.client_mcp.wait_ready_timeout_ms`; если он не задан,
  ожидание длится пять минут. Это единственная его граница: срока у команды нет.
- Если настроено `tools.client_mcp.extension`, `launch mcp` не устанавливает и не обновляет его;
  подготовка выполняется командой `v8-runner push`.
- `--mcp-config` не должен содержать `;`, потому что `/C` payload разделяется точкой с запятой.
- `launch mcp` не принимает `--c` и `--execute`, потому что `/C` управляется командой.
- Для локальной проверки external EPF используйте только `launch thin --execute <file.epf> --output <out> --stderr-output <stderr> --wait-for-exit --wait-timeout-ms <ms>`: это opt-in bounded wait с JSON-полями PID, execute path, exit code/timeout и заявленными artifact paths. Timeout считается CLI failure и возвращает error envelope с payload после остановки группы процесса. Ненулевой exit code external EPF возвращается в JSON как наблюдаемый результат; вызывающий runtime gate обязан проверить `external_epf_wait.exit_code`. Обычный `launch` остаётся асинхронным. В wait-режиме запрещены raw `/C`, `/Execute` и `/Out` (включая configured additional launch keys).
- `launch mcp` принимает общие launch flags `--use-privileged-mode`, `--output` и `--raw-key`, но
  `--raw-key` не может задавать `/C`, `/Execute` или `/Out`.
- Для `designer`/`thin`/`thick`/`ordinary` дополнительные typed flags: `--c`, `--execute`, `--use-privileged-mode`, `--output`,
  повторяемый `--raw-key`.
- Platform discovery использует `tools.platform.path` как explicit-only границу: если path задан,
  default roots и `PATH` не используются. `tools.platform.version` без path фильтрует обычный
  поиск; вместе с path проверяется только при `tools.platform.strict: true`, а при
  `strict: false` игнорируется.
- JSON-результат именно `launch` содержит legacy `binary` и `platform_resolution` с canonical
  `path`, `version` (или `null`), `source` (`explicit`, `default-root` или `path`) и
  `installation_root`. Это не общий metadata contract для остальных команд.
- `provider_dispatched` присутствует всегда и отвечает ровно на один вопрос: дошёл ли запуск до
  spawn клиентского процесса.
- `--dry-run` валидирует запрос, выбирает платформу и возвращает `provider_dispatched=false`,
  `pid=null` и compact `plan` с `program` и уже составленным `args`, но не запускает клиент.
  Превью обязано назвать провайдера, потому что платформу ищет только runner: вызывающая сторона
  не может составить эти аргументы сама.
- В `plan.args` каждое значение credential замаскировано как `***`: значение ключа `/P`, `/WSP`
  или `/AccessToken`, сегмент `Pwd=`, `wsp=`, `wsppwd=` внутри connection string и любое известное
  значение `infobase.password`, где бы оно ни встретилось. Остальные аргументы остаются читаемыми.
- То же маскирование применяется к тексту отказа и к строке журнала, когда запуск не удался: показ
  команды составляет один владелец правила. В отказе и в журнале скрыто и имя пользователя — они
  уходят в CI и живут дольше запуска, тогда как превью человек запросил сам.
- `--dry-run` несовместим с `--wait-for-exit` и `--wait-ready`: оба сообщают исход работающего
  клиента, которого превью не запускает.

### `mcp serve`

```bash
v8-runner mcp serve stdio
v8-runner mcp serve http
```

- `stdio` и `streamable HTTP` публикуют один и тот же набор из 8 инструментов.
- MCP request fields используют `camelCase`.
- Business failures возвращаются внутри tool result payload.
- Transport/internal failures остаются MCP-native.
- Все tool calls разделяют `mcp.execution.max_concurrent_calls`.
- Если пользователь просит функциональные `.feature`-сценарии, приемку или Vanessa Automation,
  агент должен выбирать `run_all_tests` с `runner=vanessa` либо `launch_app` с
  `utilityType=mcp`, `mcpScenario=va` и `waitReady=true`; bare `utilityType=mcp` не загружает
  Vanessa.

### Опубликованные MCP tools

| Инструмент | Основные поля запроса | Примечания |
| --- | --- | --- |
| `build_project` | `fullRebuild`, `sourceSet` | `fullRebuild=false`; `sourceSet` omitted значит все source-set |
| `run_all_tests` | `full`, `runner`, `profile`, `feature`, `filterTag`, `ignoreTag`, `scenarioFilter` | Компактный вывод по умолчанию; `runner=vanessa` запускает Vanessa Automation с выбранным профилем и фильтрами |
| `run_module_tests` | `moduleName`, `full` | Отклоняет пустой `moduleName` |
| `dump_config` | `mode`, `extension`, `objects` | Пустой `mode` нормализуется в `INCREMENTAL` |
| `launch_app` | `utilityType`, `mcpScenario`, `mode`, `mcpConfig`, `mcpPort`, `waitReady`, `via` | `utilityType=mcp` запускает client MCP; `mcpScenario=va` загружает Vanessa Automation; остальные MCP-поля доступны только для `utilityType=mcp`; `via` (`web` или `connection`) выбирает адрес и принимается только у тонкого клиента |
| `check_syntax_edt` | `projectName` | Пустой `projectName` значит “все EDT-проекты” |
| `check_syntax_designer_config` | Designer-config flags в `camelCase` | Область расширений нормализуется в service layer |
| `check_syntax_designer_modules` | Designer-modules flags в `camelCase` | Область расширений нормализуется в service layer |

## workPath и артефакты выполнения

Важные runtime директории:

- `workPath/hash-storages/`: persisted change-detection state.
- `workPath/edt-workspace/`: общий EDT workspace для `infobase create`.
- `workPath/convert/edt-workspace/`: отдельный EDT workspace для `convert`.
- `workPath/ibcmd-data/`: изолированный standalone-server data directory для IBCMD dump; это runtime state `v8-runner`, его можно удалить, когда нет активных CLI/MCP команд проекта.
- `workPath/logs/platform/`: platform logs.
- `workPath/logs/mcp/actions.log`: MCP action log.
- `workPath/temp/`: временные run artifacts и диагностические файлы.

## Пока не поддерживается

- Публикация CLI-only команд в MCP без отдельного ADR.
- Object-scoped partial dump через `ibcmd`.
- `upload` через `ibcmd`.
- Arbitrary path-based `convert source -> target` contract.
- Отдельная пользовательская настройка EDT `working-directory`.
