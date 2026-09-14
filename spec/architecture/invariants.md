# Архитектурные инварианты

Этот документ фиксирует правила, которые должны оставаться верными при развитии `v8-runner`.
Если изменение нарушает инвариант, сначала нужен новый ADR, который явно заменяет или уточняет текущее решение.
Практический checklist для изменений MCP surface, public command boundary и config contract вынесен в [spec/architecture/change-checklist.md](change-checklist.md).

## Цель продукта

1. Главная цель `v8-runner` — предоставить простой и удобный интерфейс для сборки и проверки исходников 1С-решения человеком и AI-агентом.
2. Основной пользовательский цикл — `build -> syntax/test -> diagnose`.
3. Новая функциональность должна упрощать этот цикл или явно объяснять, какую диагностическую, эксплуатационную или интеграционную задачу она закрывает.
4. Низкоуровневые детали утилит 1С не должны становиться обязательным знанием для обычного пользователя или AI-агента, если их можно скрыть за стабильным CLI/MCP контрактом.
5. Удобство для человека и пригодность для AI-агента являются равноправными критериями продукта.

## Публичные поверхности

1. CLI и MCP являются разными публичными поверхностями.
2. MCP не зеркалит CLI автоматически.
3. Текущая MCP-поверхность состоит из 8 tool-операций: `run_all_tests`, `run_module_tests`, `build_project`, `dump_config`, `launch_app`, `check_syntax_edt`, `check_syntax_designer_config`, `check_syntax_designer_modules`.
4. Добавление, удаление или переименование MCP tool-операций является изменением публичного контракта и требует отдельного ADR или явного обновления действующего ADR.
5. Любое изменение MCP surface должно проходить по checklist из `spec/architecture/change-checklist.md` и синхронизировать docs/source/tests, перечисленные в `ADR-0005`.
6. CLI-only команды являются допустимой частью public surface; наличие CLI-команды не должно использоваться как аргумент для неявной публикации MCP tool.

См. [ADR-0005](../decisions/0005-razdelit-cli-i-mcp-publichnye-poverhnosti.md) и [ADR-0020](../decisions/0020-dobavit-cli-only-convert-dlya-dvustoronney-konvertatsii-edt-i-designer.md).

## Config Contract

1. `v8project.yaml`, загруженный в `AppConfig` и прошедший `config::validate`, является главным конфигурационным контрактом проекта.
2. `infobase.connection` является обязательным supported ключом строки подключения; top-level `connection` не является публичным контрактом.
3. `infobase.user/password` являются supported ключами пользователя ИБ; top-level `credentials` не является публичным контрактом.
4. `infobase.dbms` описывает DBMS-level доступ для server-based ИБ; для `builder=IBCMD` + server connection обязательны `kind`, `server` и `name`.
5. Полный `infobase.dbms` contract является достаточным explicit authorization для server infobase provisioning path в `init` при `builder=IBCMD`; отдельный `tools.*` field для этого не требуется.
6. Для `builder=DESIGNER` автоматическое создание server-based ИБ в `init` не вводится; server create step остаётся explicit gap вне `IBCMD` provisioning path.
7. `infobase.dbms` не должен задаваться для file-based ИБ.
8. `source-set[].type` является поддержанным ключом типа source-set; legacy `purpose` не является публичным контрактом.
9. `source-set.name` является stable identity для ordering, diagnostics, runtime contexts, generated directories и selection logic.
10. `source-set.name` должен быть уникальным и безопасным path segment; resolved paths должны быть уникальны после normalization.
11. EDT/external source-set paths и generated work targets не должны пересекаться; reserved work directory names нельзя использовать как EDT source-set names.
12. Unsupported или unsafe config combinations должны отклоняться на validation boundary до вызова platform DSL.
13. Новый public config field, `source-set` type или `infobase` subtree требует typed model, validation, `config init`/examples/docs sync и regression tests по checklist из `spec/architecture/change-checklist.md`.
14. `v8project.local.yaml` является optional local overlay рядом с primary config, применяется после `v8project.yaml` и до CLI overrides, не является самостоятельным `--config` entrypoint и не должен менять `source-set`, `format` или `builder`.
15. `basePath` не является public key в `v8project.yaml`; внутренний project base path считается равным каталогу primary config.
16. Tool extensions, включая `tools.client_mcp.extension`, не являются project `source-set`; их подготовка выполняется через общий механизм подготовки расширений на стадии `build`, а не на стадии `launch`.

См. [ADR-0017](../decisions/0017-v8project-yaml-source-set-kak-glavnyy-konfiguratsionnyy-kontrakt.md), [ADR-0018](../decisions/0018-perenesti-kontrakt-informatsionnoy-bazy-v-infobase.md), [ADR-0019](../decisions/0019-sozdavat-servernuyu-infobazu-cherez-ibcmd-pri-init-pri-otsutstvii.md), [ADR-0021](../decisions/0021-lokalnyy-overlay-config.md) и [ADR-0022](../decisions/0022-universalnyy-mehanizm-podgotovki-rasshireniy-i-client-mcp-extension.md).

## Providers

1. Выбор исполнителя пооперационный: матрица `(операция, вид цели) → цепочка провайдеров` живёт в `domain/capability.rs` и является единственным источником для валидации, выбора перед запуском и таблицы в `docs/CAPABILITIES.md`.
2. Провайдеры закрытым набором: `designer`, `agent`, `ibcmd`, `ibcmd-rs`, `webinst`. Онлайн- и офлайн-формы `ibcmd` — следствие вида цели, а не отдельные провайдеры.
3. Цепочка умолчаний назначается владельцем проекта; `evidence` строки записывается как улика и не является воротами. Провайдер с `implementation: experimental` в цепочку умолчаний не входит.
4. `providers.<операция>` в `v8project.yaml` или `v8project.local.yaml` — единственный способ переопределить провайдера: скаляр, строгий (неготовность — типизированный отказ без отката на умолчание), отклоняется для операции без развилки и для провайдера вне матрицы.
5. Провайдер не является аргументом MCP tool, CLI-флагом или аргументом вызова.
6. Квитанция о провайдере имеет одну форму у всех операций: `selected`, `origin` (`default` | `override`), `skipped[]` с причиной. Неиспользованные альтернативы не перечисляются.
7. Сессия провайдера `agent` открывается при захвате workspace lock и закрывается при его освобождении; первая команда сессии — `options set --show-prompt=no --output-format=json`; решения принимаются по `type`/`error-type`, проза `message` — улика.
8. Готовность `agent` устанавливается результатом SSH-аутентификации с настроенными учётными данными или пустой парой; `infobase.user` обязателен ровно тогда, когда он обязателен для пакетного Конфигуратора.
9. Файлы для локального провайдера `agent` не гоняются по SFTP: каталог `/AgentBaseDir` (у Конфигуратора) и `--users-data` (у `ibsrv`) — это `workPath`, и результат читается с диска по известному правилу раскладки.
10. У каждой точки входа провайдера `agent` два режима, и режим выводится из конфига, а не угадывается: `managed` — раннер поднимает процесс сам (агент Конфигуратора или `ibsrv`), владеет его жизненным циклом и флагами (свои для своего пути плюс пользовательские из конфига; пользовательские приоритетнее, конфликт — ошибка валидации); `attached` — раннер подключается к процессу, поднятому без него (`infobase.standalone.gate`, `tools.designer_agent.attach`), не добавляет и не запрещает флагов, не перезапускает его и не поднимает свой рядом.
11. Managed-процесс живёт столько, сколько владение `workPath` (у MCP-сервера — между вызовами до shutdown); недоступный attached-процесс — типизированный отказ `endpoint_unreachable`, а не переход в `managed`.
12. Готовность пути проверяется одинаково в обоих режимах и перед использованием: `ibcmd --pid` — успешным `config generation-id`, шлюз и агент — SSH-сессией; из флагов запуска готовность не выводится.
13. `infobase.dbms` описывает доступ к СУБД и требуется только там, где раннер идёт в СУБД напрямую (создание серверной информационной базы), а не для обычной работы с существующей базой.
14. Строка подключения описывает, как клиент доходит до базы, и не задаёт вид цели: за `ws=…` может стоять файловая база, кластер или автономный сервер. Вид цели объявляется явно — `File=`, `Srvr=…;Ref=…` или `infobase.standalone`; `ws=` в `infobase.connection` не принимается.
15. У каждой цели два адреса: административный, по которому работает раннер, и клиентский `infobase.web.url`, по которому базу открывают. У автономного сервера клиентский адрес известен сразу, у файловой и кластерной появляется после публикации.
16. Секреты — пароли информационной базы и СУБД, значение `/P`, сегменты строки подключения — маскируются во всех выводах: превью, квитанциях и журналах, в текстовом и в JSON-виде.
17. Пункты разделов выше, говорящие о `builder`, действуют до реализации ADR-0030; по её завершении `builder` снимается, а эти пункты переписываются в терминах матрицы.

См. [ADR-0030](../decisions/0030-provaydery-po-operatsiyam-s-umolchaniyami-v-kode.md).

## Workspace Lock

1. Любая CLI/MCP команда, которая читает или пишет runtime state под `workPath`, должна владеть workspace lock на время выполнения.
2. Workspace lock берётся по canonical `workPath`.
3. Lock sidecar является diagnostic-only metadata; отсутствие или ошибка записи sidecar не отменяет сам lock.
4. Вложенная orchestration использует explicit internal `*_unlocked` entrypoints только под внешним lock.
5. MCP admission limits не заменяют workspace lock: semaphore ограничивает общую нагрузку, lock сериализует доступ к конкретному `workPath`.
6. Новая public CLI/MCP команда, работающая с runtime state под `workPath`, должна брать lock на adapter boundary и иметь regression coverage на boundary conflict.

См. [ADR-0011](../decisions/0011-eksklyuzivnoe-vladenie-workpath-na-vremya-komandy.md).

## MCP Admission And HTTP Sessions

1. MCP tool calls проходят через общий execution admission boundary.
2. `mcp.execution.max_concurrent_calls` ограничивает одновременно допущенные MCP tool executions для stdio и HTTP.
3. MCP admission не заменяет workspace lock и не является HTTP session capacity.
4. `mcp.http.max_sessions` ограничивает tracked stateful HTTP sessions, а не command execution.
5. HTTP initialize должен использовать reservation/confirm/release flow; overload возвращает `503`, а stateful non-initialize POST без `Mcp-Session-Id` возвращает `400`.
6. MCP cancellation/deadline должны маршрутизироваться в общую execution policy из ADR-0014.

См. [ADR-0013](../decisions/0013-mcp-execution-admission-timeout-cancellation-routing-i-http-session-capacity.md).

## Command Timeout And Cancellation

1. Timeout/cancellation являются общим CLI/MCP command contract, а не MCP-only behavior.
2. Каждая public CLI/MCP команда должна иметь execution deadline.
3. Nested orchestration наследует оставшийся budget outer command.
4. Команда не считается cancelled/timed out наружу, пока underlying operation не доведена до terminal state.
5. Operations должны иметь interruption safety class: `Interruptible`, `GracefulThenKill`, `CriticalNonAbortable` или `NoExternalProcess`.
6. Mutating DB operations после входа в critical phase не hard-kill by default; cancellation/timeout recorded и команда ждёт terminal outcome.
7. Cancellation policy живёт на command boundary; use case pipeline проверяет cancellation/deadline в safe points и не обязан моделировать отдельное cancellation state на каждом step.
8. `ExecutionStatus::Cancelled` используется только для фактической terminal cancellation.
9. Если cancellation/shutdown/timeout пришёл в critical phase, но operation безопасно завершилась success, итог остаётся `Succeeded`, а result содержит warning/diagnostic о deferred interruption.

См. [ADR-0014](../decisions/0014-edinaya-timeout-cancellation-policy-dlya-cli-i-mcp-komand.md).

## Dump And Artifacts Publication

1. Full-replacement `dump` и `artifacts` publication не должны писать напрямую в существующий target.
2. Full dump и package/external artifacts publication должны идти через staging path рядом с target и backup старого target.
3. Platform failure до publish должен сохранять старый target.
4. Publish failure должен пытаться rollback backup -> target и surfaced rollback context, если восстановление не удалось.
5. Cleanup backup/staging после успешного publish выполняется best-effort; cleanup failure становится warning/degraded success, а не failed publish.
6. `dump incremental` и `dump partial` являются non-atomic update modes и не получают staging replacement guarantee.
7. Orphan cleanup должен удалять только stale v8-runner staging/backup paths с matching target identity.

См. [ADR-0015](../decisions/0015-atomarnaya-publikatsiya-dump-artifacts-cherez-staging-backup.md).

## Infobase Export Capabilities

1. Source `dump`, source-built `artifacts`, configuration package export и DT snapshot являются разными use case.
2. Provider selection независимо сообщает implementation, current-environment readiness и evidence; только `implemented + ready` допускается к execution.
3. Для infobase export `builder` задаёт preferred provider; runner может выбрать alternate только до первого spawn. Семантика существующих команд не меняется.
4. После первого spawn provider не переключается ни при каком исходе.
5. Provider возвращает private staged file, а final output публикует общий owner по ADR-0015.
6. DT не называется backup; IBCMD DT недоступен без доказанного exclusive-access contract.
7. `implemented` не выдаётся за live proof: evidence отдельно различает documented, argv-tested и live-verified состояния.
8. Capability/readiness selection не создаёт runtime-файлы и предшествует workspace/target locks; отсутствие adapter и неготовое окружение имеют разные stable error codes.
9. `infobase ... --dry-run` использует тот же transport-neutral provider selection, что apply,
   и завершается до action logging, workspace/target locks, staging, output parent и provider process.

См. [ADR-0024](../decisions/0024-tipizirovat-eksport-konfiguratsii-i-snimka-ib.md).

## Client Launch Preview

1. Платформу для запуска выбирает только runner; отдельной команды discovery нет, поэтому превью обязано назвать выбранную программу и составленные аргументы.
2. `launch --dry-run` проходит ту же валидацию и тот же поиск платформы, что обычный запуск, и завершается до взятия runner-а и любого spawn.
3. `provider_dispatched` присутствует в результате `launch` всегда и отвечает ровно на один вопрос: дошёл ли запуск до spawn клиентского процесса.
4. В `plan.args` не попадает ни одно значение credential: маскируются значение ключа `/P`, сегмент `Pwd=` внутри connection string и известное значение `infobase.password`. Остальные аргументы остаются читаемыми.
5. Превью несовместимо с опциями, сообщающими исход работающего клиента (`--wait-for-exit`, `--wait-ready`): план не выдаётся за наблюдение.
6. Превью остаётся CLI-возможностью; MCP surface им не расширяется.

См. [ADR-0025](../decisions/0025-nevypolnyayuschee-prevyu-zapuska-klienta.md).

## Infobase Restore

1. `infobase restore` — парная операция к `infobase dump`; она загружает ИБ вместе с данными из DT и не является восстановлением из резервной копии.
2. Ровно один режим цели (`--create` или `--replace`) обязателен: ни один провайдер не спрашивает перед созданием или перезаписью ИБ, поэтому шлюз ставит runner.
3. Режим, не совпавший с наблюдаемой целью, — отказ, а не молчаливый переход к другому случаю. Наличие файловой цели определяется файлом `<infobase>/1Cv8.1CD`; серверная ИБ без процесса не наблюдается, и там режим принимается на слово вызывающего.
4. Проверка цели идёт до выбора провайдера и повторяется под workspace lock: staging-шага у загрузки нет, и отменить её нечем.
5. У загрузки нет атомарной публикации по ADR-0015; упавший провайдер даёт `target_state=uncertain`, потому что объём уже заменённых данных не наблюдаем.
6. Implemented provider — Designer; IBCMD restore остаётся `experimental` до проверки отсутствия активных сеансов, и недиспетчеризуемый адаптер для него не добавляется.
7. Принудительное завершение сеансов не публикуется, пока расхождение возможностей провайдеров не описано отдельным решением.

См. [ADR-0026](../decisions/0026-zagruzka-informatsionnoy-bazy-iz-dt.md).

## Infobase Extension Composition

1. Состав расширений информационной базы и extension `source-set` рабочего пространства — разные предметы; совпадение имени команды их не объединяет.
2. Семейство IBCMD-only: у Designer нет батч-ключа, перечисляющего установленные расширения, поэтому `builder` здесь ничего не выбирает, а отсутствие `ibcmd` — отказ, не переход на альтернативу.
3. Вывод платформы текстовый и разбирается runner-ом fail-closed: запись без обязательного поля и `yes/no`-поле с иным значением отклоняются, а не получают умолчание.
4. Пустое поле платформы означает отсутствующее значение, а не пустую строку.
5. Порядок записей не обещается: платформа выдаёт их не в порядке создания и порядок не документирует.
6. Чтение по имени проверяет, что ответ про запрошенное расширение; ответ про другое — `invalid_output`.
7. Префикс имён на чтении недоступен: платформа его не сообщает, он достаётся только выгрузкой расширения.
8. Чтение и запись состава делят одну границу workspace lock.

См. [ADR-0027](../decisions/0027-sostav-rasshireniy-informatsionnoy-bazy.md).

## Command Preview

1. Превью есть у каждого глагола, который поднимает платформу или трогает базу; оно остаётся CLI-возможностью.
2. Форм превью две: экспортный конверт там, где есть выбор провайдера, и `provider_dispatched` плюс предмет глагола там, где выбора нет. Набор кандидатов из одного элемента не вводится — выдуманная развилка хуже её отсутствия.
3. У каждой формы свой закрытый признак, и он присутствует всегда: `mode` у экспортной, `provider_dispatched` у формы `launch`. Отсутствие поля не приходится читать как «ничего не запускали» ни в одной из форм.
4. Превью возвращается после поиска утилиты: отсутствующая платформа отказывает до одобрения плана.
5. Превью не берёт workspace lock и не ждёт его: иначе «покажи план» упиралось бы в занятое пространство. `--clean-before-execution` с превью отклоняется, а не пропускается.
6. Превью не создаёт ни целей, ни артефактов, ни staging, ни рабочих пространств, ни состояния обнаружения изменений. Запись в журнале действий остаётся, как у любого запуска: она не меняет предмет.
7. Зеркальные коллекции «planned» против «produced» не вводятся: в превью поля называют планируемое, а различает режим закрытый признак формы.
8. То, что превью узнать не может, называется отдельным значением, а не заполняется умолчанием: `not_probed` у совместимости, отсутствие различия «создана/уже была» у серверной ИБ.

См. [ADR-0028](../decisions/0028-prevyu-u-glagolov-bez-vybora-provaydera.md).

## Pipeline Execution Outcome

1. Runner-like и pipeline-like сценарии должны использовать `ExecutionOutcome<T>` как canonical domain outcome для статуса, structured errors, diagnostics, metrics, artifacts and typed payload.
2. Domain result structs may keep command-specific context, but legacy compatibility fields for data already expressed by `ExecutionOutcome<T>` belong at the CLI/MCP adapter boundary.
3. Pipeline composition живёт в use case слое; CLI/MCP adapters не собирают и не исполняют pipeline blocks.
4. Blocks обмениваются typed context/input/output, а не hidden global state.
5. Значимые pipeline blocks должны иметь step entry; минимальная текущая форма `StepResult` должна эволюционировать к richer `ExecutionStep` перед массовым добавлением новых combinations.
6. `ExecutionOutcome<T>` не заменяет shared command envelope, MCP request DTO/compatibility data или `UseCaseFailure<T>`.
7. Timeout/cancellation statuses in outcome должны следовать terminal-state semantics из ADR-0014.
8. Cancellation representation остаётся command-level: `ExecutionStatus::Cancelled` для фактической отмены и diagnostic/warning для deferred interruption при successful critical phase.
9. Не вводить generic pipeline engine до появления повторяемой необходимости; сначала стандартизируются vocabulary, step contract and outcome shape.

См. [ADR-0016](../decisions/0016-edinyy-executionoutcome-i-pipeline-steps-dlya-runner-like-stsenariev.md).

## Load Compatibility Probe

1. Публичные состояния compatibility probe: `supported` (вопрос задан и доказан),
   `absent` (расширение доказано отсутствует по составу ИБ), `not_established`
   (задан и не доказан), `not_probed` (не задавался).
2. Состояние probe определяется структурно и никогда формулировкой `/Out`: у
   расширения — составом ИБ из `ibcmd config extension list`, у конфигурации —
   нулевым кодом выхода сравнения с конфигурацией поставщика, имя которой обязан
   назвать вызывающий. Закрытый allowlist целых чистых диагностик, разрешённый
   ранее, отменён вместе с пунктами 3 и 4 ADR-0023.
3. `not_established` не разрешает изменяющую операцию ни в одном режиме.
4. Матрица режима загрузки и состояния перечисляется исчерпывающе, без
   default-permit arm.
5. Старый или нечитаемый `/Out` не является доказательством состояния.

См. [ADR-0023](../decisions/0023-fail-closed-sostoyaniya-proverki-zagruzki.md) и
[ADR-0029](../decisions/0029-proza-instrumenta-ne-prinimaet-resheniy.md).

## Tool Output Contract

1. Проза инструмента не является входом решения — ни подстрока, ни целая строка, ни
   зафиксированная формулировка, ни на одном языке, включая наблюдённый.
2. Решение принимается по тому, что инструмент гарантирует: код выхода; появление
   обещанного артефакта; вывод, документированный как машинный (ключи, перечислимые
   значения, JSON). Где такой ответ доступен, используется он.
3. Проза доносится до вызывающего как улика и не интерпретируется.
4. Нет структурного ответа — факт берётся у вызывающего, а неизвестность называется
   отдельным значением; неизвестное никогда не означает успех и не разрешает
   изменение.
5. Запрет действует и на собственную прозу раннера: решение не снимается с текста
   сообщения, которое он сам построил.
6. Помета на находке вердиктом не является и допускается только там, где вердикт
   берётся из кода выхода, неопознанная строка считается ошибкой, а проза может
   сделать вердикт только строже.
7. Страж `tests/tool_output_contract.rs` находит такие решения в слоях `platform`,
   `parsers`, `use_cases` в четырёх формах — сравнение, ветвь `match`, именованная
   константа, список литералов под текстовым предикатом — и держит два списка:
   `PROSE_DEBT` (вердикт на прозе, может только сокращаться) и
   `LABELS_NOT_VERDICTS` (пометы, с доводом на каждую).

См. [ADR-0029](../decisions/0029-proza-instrumenta-ne-prinimaet-resheniy.md).

## Use Case Layer

1. `src/use_cases` остается транспортно-нейтральным orchestration-слоем.
2. Use case не зависят от `clap`, CLI `Presenter`, shared command envelope, MCP DTO и конкретного transport payload format.
3. CLI и MCP адаптеры преобразуют свои входные DTO/аргументы в `use_cases::request::*`.
4. Presentation, envelope rendering и MCP tool payload formatting остаются за пределами use case.

См. [ADR-0006](../decisions/0006-sohranyat-transportno-neytralnyy-use-case-sloy.md).

## Change Detection And Partial Load

1. Change detection выполняется on-demand во время build/export/load decision, без background watcher.
2. Persistent state хранится в per-context `redb` storages под `workPath/hash-storages`.
3. Для `format=DESIGNER` используется один `designer-<sourceSetName>` context на source-set.
4. Для `format=EDT` используются два context на source-set: `edt-<sourceSetName>` для export decision и `designer-<sourceSetName>` для load decision.
5. Recoverable scan/storage ошибки должны деградировать в full execution или full rescan; hard storage и concurrent generation errors должны surfaced as failures.
6. Partial load является conservative file-level strategy: `Configuration.xml`, deletions, unsafe expansion, empty expanded set или превышение threshold ведут к full load.
7. Prepared snapshot коммитится только после successful platform export/load step.

См. [ADR-0012](../decisions/0012-on-demand-change-detection-i-faylovaya-partial-load-strategiya.md).

## Shared EDT

1. EDT execution имеет два целевых режима: one-shot и shared interactive.
2. `tools.edt_cli.interactive_mode=false` (YAML key `interactive-mode`) означает one-shot `1cedtcli` execution.
3. `tools.edt_cli.interactive_mode=true` (YAML key `interactive-mode`) означает shared interactive EDT execution через общий actor/manager и общую interactive session.
4. Non-shared interactive EDT не является долгосрочным публичным режимом; если он встречается в коде, это implementation gap.
5. Shared interactive EDT должен сохранять baseline reset/probe, restart, shutdown/restart drain, typed errors and telemetry contract.
6. `tools.edt_cli.auto_start` (YAML key `auto-start`) является eager prewarm-флагом только для long-lived shared EDT host process; на текущем этапе это MCP server.
7. CLI при `tools.edt_cli.interactive_mode=true` стартует EDT лениво при первом EDT-вызове и не должен eagerly prewarm interactive session на старте процесса команды.
8. Если shared interactive временно покрывает не все EDT-сценарии, gap должен быть зафиксирован в документации или ADR.

См. [ADR-0007](../decisions/0007-vydelit-otdelnyy-pereklyuchatel-dlya-shared-edt.md).

## Platform Backends

1. Низкоуровневые DSL для платформенных инструментов остаются в `src/platform`.
2. `DesignerDsl`, `IbcmdDsl`, `EdtDsl`, `EnterpriseDsl`, `platform::edt_session`, `platform::locator`, `platform::process` и interactive executor не должны протаскивать process details в presentation или transport adapters.
3. Orchestration вызывает backend DSL через доменные операции и анализирует `PlatformCommandResult`, но не собирает сырые process arguments выше платформенного слоя.
4. Новый backend добавляется как отдельный adapter/DSL с явными gap и матрицей поддержки.

См. [ADR-0008](../decisions/0008-derzhat-platformennye-backend-dsl-otdelno-ot-orchestration.md).

## Failures

1. Business failures и transport/runtime failures разделены.
2. Use case возвращают `UseCaseFailure<T>` с transport-neutral metadata и, где возможно, структурированным payload.
3. MCP service разделяет `McpBusinessFailure<T>` и `McpInternalError`.
4. Orchestration не знает, как CLI или MCP сериализуют ошибку наружу.

См. [ADR-0009](../decisions/0009-razdelit-business-i-transport-runtime-failures.md).

## CLI Output

1. CLI output проектируется как единый high-signal contract для человека и AI-агента.
2. Единственный публичный selector structured CLI output — глобальный флаг `--json-message`; отдельный audience/profile-переключатель не вводится.
3. При отсутствии `--json-message` CLI печатает text output.
4. User-facing path флаг должен называться `--output`, если команда публикует один основной output path; не вводить параллельные public имена `--file`, `--out` или `--output-dir` для того же смысла без нового ADR.
5. Clean success должен оставаться кратким, а ошибки, предупреждения, degraded behavior, created artifacts, пути к диагностике и следующий actionable hint не должны теряться.
6. JSON остаётся стабильным structured contract для автоматизации; его schema не меняется только из-за различения ролей потребления.
7. Use case слой не знает presentation rules и не различает роли потребителя output.
8. CLI `--json-message` и MCP `structured_content` используют общий machine-readable command envelope с core fields `ok`, `command`, `duration_ms`, `data`, `warnings`, `steps`; business failures may include optional structured `error`.
9. MCP `CallToolResult`/`isError` и transport/internal `ErrorData` остаются protocol-level behavior и не заменяются command envelope.
10. Envelope `command` использует canonical CLI command identity; MCP tool/scope identity сохраняется внутри `data`, если она нужна клиенту.
11. Live text progress для long-running stages остаётся human-readable: может включать локальное время `HH:MM:SS` как краткий префикс строки, но не печатает structured field names вроде `started_at` и не вводит JSON/progress contract.

См. [ADR-0010](../decisions/0010-razdelit-cli-output-dlya-cheloveka-i-ai-agenta.md).
