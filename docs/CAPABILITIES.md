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
| `bootstrap` | Работает без существующего конфига | Создаёт проект из существующей ИБ: config, local overlay, `.gitignore`, `src/configuration` |
| `config init` | Работает без существующего конфига | Создаёт `v8project.yaml`, sibling `v8project.local.yaml`, `.gitignore` entry, autodetect-ит supported `source-set` и aggregate external roots |
| `tools download <tool>` | CLI-only загрузка latest releases | Загружает выбранный YAxUnit, Vanessa Automation single или onec-client-mcp-devkit; обновляет local overlay для Vanessa/client MCP и при `yaxunit --sources` добавляет YAxUnit как `source-set` `tests` |
| `init` | `format=DESIGNER` + `builder=DESIGNER` | Создаёт файловую ИБ через Designer; server connection остаётся manual prerequisite |
| `init` | `format=DESIGNER` + `builder=IBCMD` | Выполняет `ensure` файловой или серверной ИБ через `ibcmd infobase create` |
| `init` | `format=EDT` + `builder=DESIGNER|IBCMD` | Готовит ИБ по правилам builder и импортирует EDT workspace |
| `extensions` | `format=DESIGNER` или `format=EDT` | Обновляет свойства extension `source-set` |
| `build` | `format=DESIGNER` + `builder=DESIGNER|IBCMD` | Выполняет incremental/full загрузку в ИБ |
| `build` | `format=EDT` + `builder=DESIGNER|IBCMD` | Экспортирует изменённые EDT `source-set`, затем грузит generated Designer output |
| `test` | Та же матрица, что и у `build` | По умолчанию запускает `build` |
| `test --no-build` | Подготовленная file/server ИБ; source-set и build tooling не требуются | Запускает выбранный test engine без build |
| `dump` | `format=DESIGNER` + `builder=DESIGNER` | Полная, инкрементальная или object-scoped partial выгрузка |
| `dump` | `format=DESIGNER` + `builder=IBCMD` | Полная и инкрементальная выгрузка; `partial` деградирует в incremental с warning; standalone-server state изолирован в `workPath/ibcmd-data` |
| `dump` | `format=EDT` + `builder=DESIGNER|IBCMD` | Reverse sync из ИБ через internal Designer snapshot и EDT import |
| `infobase configuration export` | Designer и IBCMD | Выгружает working/database configuration в `.cf` или named extension в `.cfe`; `builder` задаёт preferred provider, runner выбирает готовый до spawn |
| `infobase dump` | Designer | Выгружает полную ИБ в переносимый `.dt`; это не backup; IBCMD остаётся experimental до exclusive-access preflight |
| `convert` | CLI-only repo-aware конвертация текущих `source-set` | Не использует `builder` и не требует ИБ |
| `load` | `format=DESIGNER` + `builder=DESIGNER` | Загрузка `.cf` / `.cfe` артефактов в ИБ |
| `make` / `artifacts` | `format=DESIGNER` + `builder=DESIGNER` | Экспорт `.cf` / `.cfe` и публикация `.epf` / `.erf` |
| `syntax` | `format=DESIGNER` или `format=EDT` | Designer checks для `DESIGNER`, EDT `validate` для `EDT` |
| `infobase restore` | Не зависит от `format`; `builder` задаёт предпочтение | Загрузка полной ИБ из DT; обязателен `--create` или `--replace` |
| `launch` | Не зависит от `format` | Прямой запуск 1C utility по позиционному mode |
| MCP | `stdio` и `streamable HTTP` | Публикует 8 инструментов, уже более узкая поверхность, чем CLI |

## Превью у глаголов, работающих с платформой

Восемь глаголов принимают `--dry-run`: `infobase configuration export`,
`infobase dump`, `infobase restore`, `launch`, `convert`, `init`, `build`,
`load`, `dump` и `artifacts`.

**Форм превью две, и это не недосмотр.** У `infobase`-экспорта и `restore` есть
настоящий выбор провайдера, поэтому их превью отвечает экспортным конвертом
(`mode=preview`, `plan.provider`, `selection.candidates`). У остальных выбора нет
— провайдер задан `builder` или единственной утилитой, — и набор кандидатов с
одним элементом был бы вымышленной развилкой. Их превью отвечает парой
**`provider_dispatched` + предмет глагола**:

| Глагол | Что называет превью |
|---|---|
| `launch` | `plan.program` и составленный `plan.args` с замаскированными credential |
| `convert` | `outputs` — что и куда было бы сконвертировано |
| `init` | по шагу `status: planned` с тем, что было бы создано и чем |
| `build` | по набору исходников планируемый `mode` и причину |
| `load` | артефакт, режим, расширение; `compatibility_state: not_probed` |
| `dump` | набор, режим, целевой путь |
| `artifacts` | вид артефакта и выход, `published: false` |

- `provider_dispatched` присутствует **всегда**, в обоих режимах: отсутствие поля
  никогда не приходится читать как «ничего не запускали».
- Превью возвращается **после** поиска утилиты: отсутствующая платформа
  отказывает до одобрения плана, а не после.
- Превью не берёт ни одной блокировки файлов и ничего не создаёт — ни целевых
  каталогов, ни EDT-рабочего пространства, ни состояния обнаружения изменений.

**Названные пределы, а не умолчания:**

- `load` возвращается до зонда совместимости, потому что зонд сам запускает
  конфигуратор. Поэтому состояние совместимости — `not_probed`, и это отдельное
  значение от `unknown`: «не спрашивали» и «спросили и не получили ответа» —
  разные факты для того, кто решает, применять ли.
- `init` для серверной ИБ не различает «создана» и «уже была»: это различие даёт
  сама `ibcmd infobase create`, то есть действие. Превью называет цель и утилиту
  и на этом останавливается.
- `build` в формате EDT планирует шаг экспорта целиком: выгрузка в файлы
  конфигуратора и последующая загрузка в базу не разделяются, потому что вторая
  зависит от результата первой.

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

### `config init`

```bash
v8-runner config init [--force] [--output <FILE>] [--connection <CONNECTION>] [--format <auto|designer|edt>] [--builder <DESIGNER|IBCMD>]
```

- Не требует существующего `v8project.yaml`.
- Пишет результат в текущий каталог или в `--output`.
- Рядом с primary config создает/обновляет пустой `v8project.local.yaml` со schema modeline и
  добавляет `v8project.local.yaml` в `.gitignore`, если подходящий pattern еще не указан.
- Не использует глобальный `--config` как shortcut output path.
- Ищет supported `DESIGNER` / `EDT` `source-set` по marker files и их содержимому.
- Для external roots создаёт aggregate `source-set` только при однородной классификации каталога.
- Не пишет synthetic `CONFIGURATION`: отсутствие конфигурационного source-set это validation error.
- Для `--builder IBCMD` найденные external roots считаются validation error.

### `bootstrap`

```bash
v8-runner bootstrap --connection <CONNECTION> --platform-version <VERSION> [--project-dir <DIR>] [--source-dir <DIR>] [--user <USER>] [--password <PASSWORD>] [--platform-path <PATH>] [--force]
```

- Работает до загрузки `v8project.yaml` и предназначен для пустого project directory.
- Создаёт `v8project.yaml`, schema-modelined `v8project.local.yaml`, `.gitignore` entry и
  `source-set main` типа `CONFIGURATION`.
- Выгружает основную конфигурацию из указанной ИБ в `src/configuration` через Designer full dump.
- `--connection` не должен содержать embedded credentials; используйте `--user` и `--password`.
  Эти значения пишутся только в `v8project.local.yaml`.
- Не обнаруживает и не выгружает расширения автоматически.

### `init`

```bash
v8-runner init [--dry-run]
```

- Всегда разделяет шаг подготовки ИБ и шаг EDT workspace.
- Для file connection и `builder=DESIGNER` использует `1cv8 CREATEINFOBASE`.
- Для `builder=IBCMD` использует `ibcmd infobase create`; server path добавляет
  `--create-database`.
- Для benign `already exists` при `IBCMD` возвращает non-fatal outcome.
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
  `build/tools/onec-client-mcp-devkit/exts/client-mcp`; без `--sources` требует
  `builder=DESIGNER` и скачивает `.cfe` в `build/tools`.
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
v8-runner extensions [--name <SOURCE_SET>...]
```

- Работает только с `source-set`, у которых `type=EXTENSION`.
- Без `--name` обрабатывает все extension `source-set` из конфига.
- Возвращает пошаговый результат по каждому целевому расширению.

#### Состав расширений информационной базы

```bash
v8-runner extensions list
v8-runner extensions info --name <NAME>
v8-runner extensions create --name <NAME> --name-prefix <PREFIX> [--synonym <NSTR>] [--purpose <customization|add-on|patch>]
v8-runner extensions delete --name <NAME>
v8-runner extensions activate --name <NAME> --active <yes|no>
```

- Предмет здесь другой: `extensions` без подкоманды правит свойства extension
  `source-set` рабочего пространства, а подкоманды читают и меняют состав
  расширений, **установленных в информационной базе**. Это разные вещи, и
  совпадение имён их не объединяет.
- **Семейство IBCMD-only.** У Designer нет батч-ключа, который перечисляет
  установленные расширения, поэтому `builder` здесь ничего не выбирает; при
  отсутствии `ibcmd` операция отказывает, а не уходит на Designer.
- `list` и `info` отдают по расширению: `name`, `version`, `active`, `purpose`,
  `safe_mode`, `security_profile_name`, `unsafe_action_protection`,
  `used_in_distributed_infobase`, `scope`, `hash_sum`. Пустое поле платформы —
  отсутствующее значение, а не пустая строка: в JSON его просто нет.
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

### `build`

```bash
v8-runner build [--source-set <NAME>] [--full-rebuild] [--dry-run]
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
  `tools.client_mcp.extension.name`.
- Для source-backed `tools.client_mcp.extension` использует отдельное состояние change detection
  под `workPath/hash-storages`: неизменённый source пропускает export/load, `--full-rebuild`
  принудительно обновляет расширение.
- `tools.client_mcp.extension` не является project `source-set`; `--source-set` выбирает только
  project source-set.
- Не является атомарной multi-source-set операцией: ранние успешные шаги не откатываются, если
  поздний шаг падает.

## Проверка и валидация

### `test`

```bash
v8-runner test [--full] [--no-build] yaxunit all
v8-runner test [--full] [--no-build] yaxunit module <NAME>
v8-runner test [--no-build] va
v8-runner test [--no-build] va --feature login --filter-tag @smoke
```

- По умолчанию сначала запускает `build`. `--no-build` отмечает build-step как `skipped` и
  запускает тесты на подготовленной ИБ; для file connection до запуска платформы требуется
  `<infobase>/1Cv8.1CD`, для server connection доступность подтверждается запуском test engine.
- В `--no-build` source-set и build tooling не проходят filesystem/layout validation: исходники
  configuration могут отсутствовать. Валидация ИБ, платформы и настроек test engine сохраняется.
- `--no-build` является CLI-only контрактом; MCP `run_all_tests` сохраняет build-first поведение.
- `test yaxunit module <NAME>` требует непустое имя модуля.
- `test va` использует профиль из `tests.va.profile`; `--feature`, `--filter-tag`,
  `--ignore-tag` и `--scenario-filter` переопределяют соответствующие списки выбранного профиля
  только для текущего запуска.
- Для функциональных `.feature`-сценариев и приемки используйте Vanessa Automation: CLI
  `test va` или MCP `run_all_tests` с `runner=vanessa`, а не дефолтный YaXUnit-runner.
- `--full` включает полный вывод успешных кейсов и расширенные stack traces.
- `tests.*.timeouts.total_ms` остаётся активным пользовательским контрактом таймаутов.

### `syntax`

```bash
v8-runner syntax designer-config [FLAGS]
v8-runner syntax designer-modules [FLAGS]
v8-runner syntax edt [--project <PROJECT>...]
```

`designer-config`:

- Только `builder=DESIGNER`, `format=DESIGNER`.
- Позволяет комбинировать config checks и client scopes.
- Поддерживает `--extension <EXTENSION>` или `--all-extensions`.

`designer-modules`:

- Только `builder=DESIGNER`, `format=DESIGNER`.
- Требует как минимум один mode flag.
- Поддерживает `--extension <EXTENSION>` или `--all-extensions`.

`edt`:

- Только `builder=DESIGNER`, `format=EDT`.
- Повторяемый `--project`.
- Без `--project` использует дефолтный набор EDT-проектов из конфига.

## Файлы и артефакты

### `dump`

```bash
v8-runner dump --mode <full|incremental|partial> [--source-set <NAME>] [--extension <EXTENSION>] [--object <TYPE:NAME>...] [--dry-run]
```

- `partial` требует хотя бы один `--object`.
- Канонический ввод селектора — `TYPE:NAME` (например, `Catalog:Items`); для
  совместимости принимается и `TYPE.NAME`. Переданный селектор сохраняется в JSON как
  `data.selectors[*].requested`, а в списке Designer и как
  `data.selectors[*].normalized` используется нормализованный `TYPE.NAME`.
- До запуска платформы CLI валидирует синтаксис селектора: непустые `TYPE` и `NAME`,
  ровно один разделитель `:` или `.`, без управляющих символов. В `builder=DESIGNER`
  существование metadata root type проверяет Designer; `builder=IBCMD` не использует object list,
  потому что деградирует в incremental.
- `builder=DESIGNER` поддерживает true object-scoped partial.
- `builder=IBCMD` не умеет object-scoped partial; запрос деградирует в incremental с warning.
- `format=EDT` использует internal Designer snapshot под `workPath/designer/<sourceSetName>`,
  затем импортирует его в EDT target и публикует результат атомарной заменой target каталога.

### `convert`

```bash
v8-runner convert [--source-set <NAME>] [--output <DIR>] [--dry-run]
```

- CLI-only; не публикуется как MCP tool.
- Работает от текущего `v8project.yaml`, а не по arbitrary source/target paths.
- Направление определяется только из `format`.
- Без `--output` публикует результат под `workPath/convert/out/<sourceSetName>/<designer|edt>/`.
- `--output` задаёт только target root и зеркалит `source-set.path` относительно каталога primary config.
- Публикация остаётся staged full replacement с overlap guardrails.

### `infobase configuration export`

```bash
v8-runner infobase configuration export --state <working|database> --output <FILE.cf> [--dry-run]
v8-runner infobase configuration export --state <working|database> --extension <NAME> --output <FILE.cfe> [--dry-run]
```

- Сохраняет состояние конфигурации из ИБ, а не собирает пакет из project sources.
- Без `--extension` экспортирует main configuration и требует `.cf`; с extension требует `.cfe`.
- `builder` задаёт preferred provider, но runner выбирает первый `implemented + ready` candidate.
- Fallback допустим только во время pure preflight; после первого spawn provider не переключается.
- Публикация идёт через sibling staging и target-specific lock; `published=true` означает, что
  финальный файл уже заменён атомарно.
- Target lock сериализует cooperating запуски runner. Параллельный внешний writer обязан
  использовать тот же lock или быть остановлен: path revalidation не является filesystem CAS.
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
- В первом slice implemented provider — Designer. При `builder=IBCMD` runner пропускает
  experimental IBCMD DT и выбирает готовый Designer.
- Если implemented provider есть, но binary/version/connection не готовы, возвращается
  `environment_unavailable`; `capability_unavailable` означает отсутствие implemented adapter.
- Для файловой ИБ readiness обоих process providers требует существующий файл
  `<infobase>/1Cv8.1CD`; один каталог или найденный бинарник не считаются готовой ИБ.
- Selection показывает каждого кандидата через независимые `implementation`, `readiness` и
  `evidence`; `argv_tested` не выдаётся за live proof.
- Безопасный IBCMD path требует проверки отсутствия активных сеансов.
- Обе операции требуют существующий `v8project.yaml`, но используют infobase-only validation:
  отсутствующий `source-set` равнозначен `source-set: []`, а сломанные project sources не блокируют чтение ИБ.
  Build/source/test/EDT/client-MCP настройки для этих команд не валидируются.

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

### `load`

```bash
v8-runner load --path <FILE> [--mode <load|merge>] [--settings <FILE>] [--extension <NAME>] [--dry-run]
```

- Поддерживает `.cf` и `.cfe`.
- Работает только для `format=DESIGNER` и `builder=DESIGNER`.
- `.cfe` требует `--extension`.
- `--mode merge` требует `--settings <FILE>`.
- Первая загрузка отсутствующего в ИБ расширения возвращает
  `compatibility_state=absent` и выполняется через `--mode load`; `merge` для
  такого расширения отклоняется с рекомендацией сначала выполнить `load`.
- Неоднозначная ошибка compatibility probe не разрешает изменяющую операцию:
  состояние остаётся `unknown`, команда возвращает platform failure, а артефакт
  не загружается. Новая локализованная формулировка принимается как `absent`
  только после фиксации реального вывода платформы и regression-теста.
- `load --mode update` не поддержан; используйте `load` или `merge`.

### `make` / `artifacts`

```bash
v8-runner make --output <TARGET> [--source-set <NAME>] [--extension <NAME>] [--dry-run]
v8-runner artifacts --output <TARGET> [--source-set <NAME>] [--extension <NAME>] [--dry-run]
```

- Это один use case с двумя CLI names.
- `.cf` используется для основной конфигурации.
- `.cfe` используется для extension export.
- Каталог output используется для external `.epf` / `.erf` publication.
- Требует `builder=DESIGNER`.

## Прямой запуск и MCP

### `launch`

```bash
v8-runner launch <designer|thin|thick|ordinary> [--dry-run] [FLAGS]
v8-runner launch mcp [va] [--mode <thin|thick|ordinary>] [--wait-ready] [FLAGS]
```

- Для обычного запуска (`designer`/`thin`/`thick`/`ordinary`) режим задаётся позиционным
  аргументом.
- `designer` использует `1cv8`.
- `thin` использует `1cv8c`.
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
  используется общий `execution_timeout`. Фактическое ожидание всё равно ограничено общим
  command deadline, поэтому для более длинного ожидания нужно увеличить и `execution_timeout`.
- Если настроено `tools.client_mcp.extension`, `launch mcp` не устанавливает и не обновляет его;
  подготовка выполняется командой `v8-runner build`.
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
- В `plan.args` каждое значение credential замаскировано как `***`: значение ключа `/P`, сегмент
  `Pwd=` внутри connection string и любое известное значение `infobase.password`, где бы оно ни
  встретилось. Остальные аргументы остаются читаемыми.
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
| `launch_app` | `utilityType`, `mcpScenario`, `mode`, `mcpConfig`, `mcpPort`, `waitReady` | `utilityType=mcp` запускает client MCP; `mcpScenario=va` загружает Vanessa Automation; остальные MCP-поля доступны только для `utilityType=mcp` |
| `check_syntax_edt` | `projectName` | Пустой `projectName` значит “все EDT-проекты” |
| `check_syntax_designer_config` | Designer-config flags в `camelCase` | Область расширений нормализуется в service layer |
| `check_syntax_designer_modules` | Designer-modules flags в `camelCase` | Область расширений нормализуется в service layer |

## workPath и артефакты выполнения

Важные runtime директории:

- `workPath/hash-storages/`: persisted change-detection state.
- `workPath/edt-workspace/`: общий EDT workspace для `init`.
- `workPath/convert/edt-workspace/`: отдельный EDT workspace для `convert`.
- `workPath/ibcmd-data/`: изолированный standalone-server data directory для IBCMD dump; это runtime state `v8-runner`, его можно удалить, когда нет активных CLI/MCP команд проекта.
- `workPath/logs/platform/`: platform logs.
- `workPath/logs/mcp/actions.log`: MCP action log.
- `workPath/temp/`: временные run artifacts и диагностические файлы.

## Пока не поддерживается

- Публикация CLI-only команд в MCP без отдельного ADR.
- Object-scoped partial dump для `builder=IBCMD`.
- `load` для `IBCMD`.
- Arbitrary path-based `convert source -> target` contract.
- Отдельная пользовательская настройка EDT `working-directory`.
