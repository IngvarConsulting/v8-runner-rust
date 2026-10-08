# Deep Dive

Этот документ описывает execution semantics и operational nuances `v8-runner` без дублирования
полного каталога команд. За точным пользовательским surface обращайтесь к
[CAPABILITIES.md](CAPABILITIES.md), за YAML-контрактом к [CONFIGURATION.md](CONFIGURATION.md).

## Навигация

- [Модель выполнения](#модель-выполнения)
- [source-set и change detection](#source-set-и-change-detection)
- [Пайплайн push](#пайплайн-push)
- [status](#status)
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
  Чужая память для `push` — отсутствие памяти, и у `--full` тоже: отказ `no_memory` называет
  её чужой; новую память создают полный `pull` (`pull <SET> --force`) или перезапись
  `push --force`.
- Кеш экспорта EDT и внешних артефактов остаётся общим в `workPath/hash-storages/`.
  Журнал поколений — `workPath/infobases/<база>/generation.json`, запись на набор с
  привязкой памяти набора, именем инструмента, которым получен токен (`designer`, `ibcmd`,
  `agent`), и операцией; загрузка без применения (`push --no-apply` или отказ применения)
  добавляет `applied: false`, а `apply` переносит запись на поколение после себя и снимает
  признак. Прежний раннер поля не знает и читает такую запись как обычную запись отправки. `push` спрашивает поколение до загрузки и после неё, `pull` — до
  выгрузки и после неё; изменившееся во время выгрузки поколение ответ называет, а в журнал
  ложится поколение до выгрузки, и следующий `push` откажет. Выгрузка по изменившемуся, перед
  которой поколение совпало с записью того же инструмента, не запускается (`up_to_date`).
  Без памяти о базе `push` (и `--full`, и превью, и `test`) отказывает `no_memory`, базу,
  ушедшую вперёд, — `non_fast_forward`; только `push --force` перезаписывает базу без этих
  проверок.
  Токен сравнивается только с токеном того же инструмента: запись другой пары, другого
  инструмента, запись без имени инструмента или неразборчивая запись не даёт пропустить
  выгрузку.
- Копия файла версий набора лежит в `workPath/infobases/<база>/dump-info/<набор>/`:
  подменённый `ConfigDumpInfo.xml` в каталоге набора перед выгрузкой по изменившемуся, перед
  выборкой `ibcmd` (`--object`, идёт как `--sync`) и перед
  загрузкой Конфигуратором или агентом заменяется ею, после удачной команды она перенимает
  файл платформы. Исключения: выборочная выгрузка Конфигуратора, `push` через `ibcmd` и `push`
  через агента, получившего копию каталога (SFTP, общий каталог без ссылки), копию не меняют — файл в каталоге они не переписывают или их запись
  остаётся на стороне агента; сбой копию тоже не меняет.
- Generated Designer output для EDT flow живёт под памятью базы
  `workPath/infobases/<база>/designer/<sourceSetName>`; внешние обработки и отчёты — под
  `workPath/designer/<sourceSetName>`.
- Файл версий и версия формата в нём читаются до запуска платформы. Нет файла или в его корне
  нет версии формата — выгрузка по изменившемуся становится полной поверх каталога, а снимок
  EDT заменяется целиком; ответ и превью называют `FULL` и причину. Лишнего такая выгрузка не
  удаляет и хеш-память не пишет (выравнивает `pull <SET> --force`).
- Любая выгрузка без `--force` до запуска платформы спрашивает сторожа замены
  (`use_cases::destruction_guard`) о каталоге набора: поверх каталога — по изменившемуся,
  выборкой, полной без файла версий — `overwrite`, замена и проект EDT — `replace` (замена ещё
  раз перед публикацией). Незакоммиченное в каталоге набора и каталог вне системы контроля
  версий с файлами — отказ до платформы; пустой каталог терять нечего. С `--force` сторож
  отвечает перечнем уничтоженного: публикация отдаёт его в `data.losses` и в сообщение. Превью
  спрашивает того же сторожа и ничего не пишет: `git status` идёт с `GIT_OPTIONAL_LOCKS=0`. `ibcmd config export` без `--sync` в непустой каталог
  отказывает, поэтому полная выгрузка `ibcmd` идёт в промежуточный каталог и ложится поверх
  каталога набора. Версия формата сверяется по таблице замеров «платформа → версия
  формата» (8.3.27 — 2.20, 8.5.4 — 2.22).
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

## `status`

`status` ничего не меняет — пишет только журналы платформы и сессии агента — и делит знание на два уровня по цене:

- без ключа — только память под `workPath/infobases/<база>`: по набору — помнит ли копия базу
  (то же определение памяти, по которому `push` отказывает `no_memory`), запись журнала
  поколений и число файлов, изменившихся с последнего чтения каталога (анализ изменений без
  записи). Платформа не запускается и может отсутствовать; `--all` повторяет это для каждой
  базы местного слоя;
- `--deep` — команда чтения базы под её замком: поколение спрашивает тот же исполнитель, что
  выбрал бы `push`, и сверка с записью идёт по тому же правилу, что сверка перед загрузкой, —
  поэтому при `moved_ahead` `push`, который грузит этот набор без `--force`, откажет `non_fast_forward`. Читает поколение тот же читатель, что у `push` и `pull`
  (`generation_reader`), сверка — `exchange_guard::predict`. Состав расширений читает
  исполнитель `extensions` без снимков ради префиксов; копии-владельцы берутся из метки рядом с
  файловой базой. Чего платформа не ответила, ответ называет `null` с причиной.

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
- `pull --all` (`dump_config::execute_all`) читает список расширений базы выбранным
  исполнителем `pull`, обходит пакеты в порядке `SourceSetInventory::configuration_packages`
  тем же сценарием `pull <SET>` и для расширения без набора выгружает его полностью в
  `src/ext/<Name>` (от `basePath`), а затем дописывает набор в `v8project.yaml`
  (`config_init::declare_source_sets`, текстом, с повторным чтением). До первой выгрузки
  проект с объявляемыми наборами проходит `config::validate::validate_with_declared_source_sets`
  — те же проверки, что при загрузке, без требования каталога, которого ещё нет.
  Расширение-инструмент `tools.client_mcp.extension` не объявляется. Объявление идёт после
  выгрузки: отказ посреди обхода не оставляет в проекте набора без содержимого, а сбой между
  выгрузкой и объявлением оставляет каталог без записи — его сторож охраняет советом
  `ForceWayOut::Undeclared` (без `pull <SET> --force`).
- `download` без набора (`infobase_export::execute_configuration_export_all`) выбирает
  исполнителя выгрузки один раз, читает им состав базы тем же читателем, что `pull --all`
  (`installed_extensions::read_installed_extensions`), сопоставляет наборы
  `SourceSetInventory::installed_packages` (со сторожем #218) и выгружает найденные пакеты
  сценарием `download <SET>` в `source_inventory::package_in_directory`. `make` без набора
  (`artifacts::execute_all`) обходит `SourceSetInventory::ordered_source_sets` сценарием
  `make <SET>`. Каталог вместо файла проверяет `SourceSetInventory::packages_directory` и
  отдаёт разрешённый путь; до работы `SourceSetInventory::check_package_targets` отказывает,
  если пакет ложится на каталог набора или `workPath` (совпадает, внутри, вокруг), если имена
  пакетов совпадают без регистра или называют устройство Windows. Накопление ответов,
  остановку и закрытие ответа у всех трёх обходов ведёт `set_walk`. Проект без пакетов
  (`source-set: []`) у `download` идёт прежним путём одной выгрузки.

### `convert`

Это repo-aware файловая конвертация между XML платформы, проектом EDT и пакетом `.cf`/`.cfe`.

- Не использует базу проекта: загрузчик её не выбирает (`load_config_without_infobase`), а
  `--infobase` отвергает `cli::global_flags`.
- Не является alias для `pull`.
- Наборы берёт из `v8project.yaml`; файл пакета — значение с расширением `.cf`/`.cfe` на
  месте набора (`ConvertScopeRequest::from_arguments`).
- Направление решает `convert_sources::resolve_direction` из `--to`, формата и вида входа.
  EDT ↔ XML исполняет `1cedtcli`; направления с пакетом — `convert_sources::package`: выбор
  исполнителя по строке `convert` матрицы (`provider_selection::select`, квитанция в
  `data.provider`), затем `ThrowawayInfobase` под `workPath/temp/throwaway-infobases/` —
  тот же владелец, что у `make`: `build_package` (`config import --out`) и `export_package`
  (`config export --file`). Исходники EDT сперва переводит в XML
  `ThrowawayInfobase::xml_from_edt` в каталог временной базы. Пакет публикуется
  заменой файла, XML — заменой каталога со сторожем незафиксированной работы; цель
  перепроверяется после работы исполнителя.

### `upload`, `make`, `artifacts`

Это materialization сценарии поверх готовых артефактов или publish targets.

- `upload` работает с готовыми `.cf` / `.cfe`.
- `make` / `artifacts` собирают final `.cf`, `.cfe`, `.epf`, `.erf` из исходников и
  публикуют их. База проекта не участвует: пакет собирается во временной базе раннера
  (`use_cases::throwaway_infobase::ThrowawayInfobase`) под `workPath/temp/throwaway-infobases/`
  — `ibcmd` (`infobase create` со своим `--data`, затем `config import --out`) или
  Конфигуратор (`CREATEINFOBASE`, `/LoadConfigFromFiles` без файла версий, `/DumpCfg`;
  расширение — поверх основной конфигурации). Исходники EDT сперва переводит в XML
  `throwaway_infobase::edt_sources_to_xml` — единственный перевод у `make`, `convert` и
  `infobase create` (временная база зовёт его через `ThrowawayInfobase::xml_from_edt`): шаг
  сборки `build_project::execute_edt_export_step` в рабочей области `workPath/edt-workspace`,
  общей сессией EDT команды, если она её держит, — тогда действует предел команды сессии
  (`tools.edt_cli.command_timeout_ms`); одноразовым процессом у `make` и `infobase create` шаг
  без предела, как у `push`, у `convert` — с пределом EDT команды. Других вызывающих шага, кроме сборки
  `push`, нет — это держит проверка `the_edt_export_step_has_one_converter_besides_push`. База служит прогону, своя у каждого исполнителя (`artifacts::MakeSession`):
  `make <SET>` — своя, обход без набора — общая на все наборы; внешние обработки Конфигуратор
  собирает поверх основной конфигурации в своей базе; после прогона она убирается, а
  брошенную описание `TempDirKind::ThrowawayInfobase` выдаёт уборке как свою. Замка базы и
  метки владельца у `make` нет.
- Full replacement target publication идёт через staged publication model.

### `infobase create`

Сценарий — `use_cases::init_project`; вид цели решает путь. Файловая база: память
собираемого набора снимается до сборки (`exchange_guard::AssembledMemory`; у проекта EDT —
исходники EDT до перевода, а основной набор переводит в XML каталога, куда его переводит
`push`, единственный перевод `throwaway_infobase::edt_sources_to_xml` после импорта рабочей
области), затем `ibcmd
infobase create --import --apply --force` или Конфигуратор (`CREATEINFOBASE`,
`/LoadConfigFromFiles` без файла версий, `/UpdateDBCfg`); после удачи
`exchange_guard::remember_created_base` пишет собранному набору его дерево, остальным —
пустую память. Сборка Конфигуратором, остановленная после создания, оставляет пустую память.
База в кластере: строку `CREATEINFOBASE` собирает `V8Connection::create_cluster_infobase_arg`
из `Srvr`/`Ref` подключения и реквизитов `dbms`/`cluster`, всегда с `CrSQLDB=Y` и
`SchJobDn=Y` (база создаётся с запретом регламентных заданий); `/Out` не ставится, а вывод
платформы в отказе проходит `mask_text` с паролями СУБД и кластера. `DBPwd` и `SPwd`
маскирует `platform::secrets`, `DBUID` и `SUsr` прячутся в показе отказа. Все процессы
создания — критическая фаза; отсрочку отмены называет `collecting_deferrals`.

С `--from` путь другой — `init_project/copy.rs`: конфигурация источника
(`copy::source_config`) — та же, что у команды, с его секцией из местного слоя. Замок
источника берёт граница — `transport::hold_source_base` из адаптера CLI, вслед за замками
своей базы, до конца команды; проверки владельца у источника нет. Снимок и загрузка образа
идут исполнителями `infobase dump` и `infobase restore` — `infobase_export::
run_snapshot_provider` и `run_restore_provider` с Конфигуратором, образ проверяет
`validate_platform_artifact`; у кластера перед загрузкой базу создаёт `ClusterCreation` —
тот же, что у `infobase create`, с тем же предупреждением о `CrSQLDB=Y`. После удачи
`exchange_guard::remember_copied_base` стирает прежнюю память под именем базы и пишет
`copied-from.json`: источник, образ, время. `memory_of` считает признак
памятью набора, `build_project` превращает первую отправку в полную, `Standing` не предлагает
выгрузку; признак снимает удачная отправка (`forget_copied_base`) и `remember_created_base`.

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
  `infobase_busy`.
- Под замком базы граница проверяет метку владельца рядом с каталогом базы: команда записи на
  базе другой рабочей копии идёт с предупреждением шага `infobase owner` и метку не меняет;
  команда записи на базе из местного слоя без другого живого владельца записывает свою копию в
  метку. Проверка одна — в `use_cases/transport.rs`, за замком базы, для CLI и MCP; превью
  читает метку без замка. Запись другой копии владелец замечает своей следующей отправкой по
  поколению (`non_fast_forward`).

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
