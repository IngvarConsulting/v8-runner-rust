# Backlog по целевой модели сайта

Документ сформирован 2026-09-21 сверкой сайта `docs/site` с кодом ветки
`design/cli-vocabulary`. Сайт описывает целевое состояние раннера и не описывает
текущее; всё, чем код от него отличается, собрано здесь и превращено в задачи.

Сверка шла четырьмя проходами по `src`: командная строка (`src/cli`), схема
конфигурации (`src/config`), исполнители и память (`src/domain`, `src/use_cases`),
покрытие операций платформы (`src/platform`). Ссылки на код даны как `файл:строка`
на момент сверки; ссылки на сайт — как `страница#якорь`; ссылки на реестр — символами
`spec/arch/index.md`.

Приоритеты те же, что в `spec/archive/ADR_DERIVED_BACKLOG_2026-04-30.md`:

- `P0`: без этого остальное не собрать, или код противоречит уже принятому решению.
- `P1`: публичный контракт сайта, которого в коде нет.
- `P2`: расширение покрытия, замеры, чистка.
- `P3`: стражи от повторного расхождения.

Каждая задача называет, что говорит сайт, что делает код, что сделать и по чему
принимать. Задачи с пометкой «замер» требуют проверки на живой платформе до
реализации; список замеров собран отдельно в разделе 5.

## 1. Сводка

| Область | Сайт | Код | Задачи |
| --- | --- | --- | --- |
| Словарь команд | `status`, `init`, `clone`, `push`, `pull`, `apply`, `upload`, `download`, `diff`, `make`, `check`, `test`, `convert`, `extensions list\|set`, `sessions`, `infobase create\|dump\|restore`, `publish`, `launch`, `tools download`, `mcp serve`; прежние имена — скрытые синонимы на один цикл (`cli.html#map`, `#synonyms`) | `bootstrap`, `config init`, `init`, `build`, `dump`, `load`, `syntax`, `infobase configuration export`, `extensions … create\|activate`; нет `status`, `clone`, `push`, `pull`, `apply`, `upload`, `download`, `diff`, `check`, `sessions`, `infobase create`; механизма скрытых синонимов нет (`src/cli/args.rs:46-82`) | A |
| Конфигурация | карта `infobases` с `origin` по умолчанию; секция базы: `connection`, `user`, `password`, `dbms`, `cluster.{ras,user,password,agent}`, `standalone.{gate,exchange}` рядом со строкой, `web`; `providers.push\|pull\|apply\|upload\|download\|diff\|make\|extensions\|infobase.*` (`cli.html#keys`) | один объект `infobase` (`src/config/model.rs:39`); `standalone` и `connection` взаимно исключают друг друга (`validate.rs:63`); секции `cluster` нет; 11 ключей `providers` со старыми именами (`capability.rs:78-100`); синонимов ключей нет, прежние написания — жёсткий отказ (`loader.rs:315`) | B |
| Цели и исполнители | автономный сервер: Конфигуратор по прямому шлюзу первым, `agent` по SSH вторым; кластер без `ibcmd`; `infobase create` — `ibcmd --import` для файловой, Конфигуратор `CREATEINFOBASE` с реквизитами СУБД для кластера, отказ с рецептом для автономного; `agent` первым в цепочке файловой базы и кластера (`architecture.html#d-ops`) | автономный сервер — только SSH-шлюз, `load`, `check`, снимок, `test`, `launch designer` отказывают (`capability.rs:284-299`); `CREATEINFOBASE` только файловый (`designer.rs:178-186`), серверную базу создаёт `ibcmd` по `dbms` (`ibcmd.rs:458-466`); `agent` экспериментален и в цепочку не входит (`capability.rs:233-237`) | C |
| Сеансы | `sessions list\|terminate\|deny\|allow` через `rac`, `apply --sessions disable\|force`, `ibcmd session` у автономного сервера (`cli.html` «Сеансы») | ни `rac`, ни `-SessionTerminate`, ни `--session-terminate` в `src` нет; `ibcmd config apply` всегда `--force --dynamic auto` (`ibcmd.rs:378-390`) | D |
| Память и первый контакт | память по базе под `workPath/infobases/<имя>/`: поколение, хеши, файл версий; отказ `non_fast_forward` с полем `next`; `push` в непустую базу без памяти отказывает (`sources.html#memory`, `cli.html#refusals`) | хеши по набору исходников, не по базе (`source_set.rs:45-50`); поколение пишет только агент (`agent_session.rs:809-867`); защиты от ушедшей вперёд базы нет; поля `next` в конверте нет (`command_envelope.rs:9-14`) | E |
| Исходники и расширения | имя, префикс и назначение расширения берутся из `Configuration.xml`, `push my-ext` заводит расширение сам; `download --state db`; `upload --mode combine\|update`; `diff --against vendor:<имя>` (`sources.html`, `cli.html`) | `extensions create` требует `--name-prefix` и `--purpose` флагами (`args.rs:346-357`); `load --mode update` отвергается валидацией (`load_artifact.rs:814`); `/CompareCfg` только как проба совместимости (`designer.rs:147-176`) | F |
| Реестр решений | целевая модель заменяет три инварианта автономной цели, решение о создании серверной базы через `ibcmd`, решение о единственной секции `infobase`; переименованные команды меняют контракты `CTR.WIRE.*` | записи активны и проверяются тестами | G |

## 2. Задачи

### A. Словарь команд

1. `SITE-TASK-001` (P0). Скрытые синонимы команд и ключей.
   Сайт: прежние имена принимаются один цикл выпуска, в справке их нет, ответ идёт под
   новым именем (`cli.html#synonyms`). Код: только видимые псевдонимы `artifacts`,
   `vanessa-automation-single`, `client_mcp` (`src/cli/args.rs:72,156,159`), скрытых нет.
   Сделать: механизм скрытых псевдонимов в `clap` (`alias` + `hide`), таблица
   «прежнее → новое» в одном месте, метка `command` конверта всегда новая
   (`src/use_cases/context.rs:30-49`); страж, что синоним не попадает в `--help`.
   Приёмка: `tests/cli_*` вызывают каждое прежнее имя и получают ответ под новым.

2. `SITE-TASK-002` (P0). Переименования: `config init → init`, `bootstrap → clone --from`,
   `build → push`, `dump → pull`, `infobase configuration export → download`,
   `load → upload`, `syntax → check`, `load --mode merge → upload --mode combine`.
   Сайт: `cli.html#map`. Код: `src/cli/args.rs:50-82`. Прежний `init` создавал базу и
   синонима не получает: под этим именем живёт другая команда (`cli.html#synonyms`),
   его работа уходит в `infobase create` (задача 020).
   Сделать: новые имена и флаги по словарю (`push --no-apply|--full|--force|--delete`,
   `pull --force|--all`, `download --state db`, `upload --mode combine|update`),
   контракты `CTR.WIRE.*` переименовать (раздел G).
   Приёмка: `--help` показывает только словарь сайта; прежние вызовы проходят по 001.

3. `SITE-TASK-003` (P1). Новые команды без сегодняшнего аналога: `status [--deep|--all]`,
   `apply [--sessions disable|force]`, `diff [--against db|vendor:<имя>|<файл>]`,
   `sessions list|terminate|deny|allow`, `infobase create`, `extensions set`.
   Сайт: `cli.html#map`, `usecases.html`. Код: отсутствуют (`src/cli/args.rs`).
   Разбиты по областям: `status` — 041, `apply` и `sessions` — D, `diff` — 052,
   `infobase create` — 020, `extensions set` — 050.

4. `SITE-TASK-004` (P1). Глобальный ключ `--infobase <имя|строка>` и глобальный
   `--dry-run`. Сайт: всякая команда с базой принимает `--infobase`, строка целиком —
   для сценария сборки без правки конфига (`cli.html` «Начало работы»); превью у всех
   команд с платформой (`index.html` словарь). Код: глобальных флагов шесть, среди них
   нет ни `--infobase`, ни `--dry-run` (`src/cli/args.rs:9-43`); `--connection` есть
   только у `bootstrap` и `config init` (`args.rs:105,208`).
   Сделать: глобальный `--infobase` с разбором «имя или строка», память о базе по
   имени или по хешу строки; `--dry-run` поднять до глобального, сохранив правило
   «превью не берёт замок» (`DEC.2026-09-11.PREVIEW-DOES-NOT-TAKE-THE-LOCK`).

5. `SITE-TASK-005` (P1). Поле `next` в отказе. Сайт: отказ называет следующий шаг
   отдельным полем, например `{"command": "pull", "source_set": "main"}`
   (`cli.html#refusals`). Код: `EnvelopeError {code, kind, message}` без `next`
   (`src/command_envelope.rs:9-14`); видов ошибок девять (`src/cli/output.rs:63-75`),
   `non_fast_forward` среди них нет.
   Сделать: расширить `CTR.WIRE.COMMAND-ENVELOPE` полем `next` и новыми видами
   (`non_fast_forward`, `no_memory`, `target_kind`), текстовую строку оставить
   человеку.

6. `SITE-TASK-006` (P1). `upload --mode update` и адресация расширения.
   Сайт: `upload <файл> --mode combine|update` объединяет по файлу настроек или
   обновляет конфигурацию поставщика (`cli.html`, тезис 49); расширение адресуется
   ссылкой. Код: `--mode update` принимается `clap` и отвергается валидацией
   (`src/use_cases/load_artifact.rs:814-818`), ветки `unreachable!` (`:296,321,338,363`);
   `--extension` вместо `--ref` (`args.rs:247`).
   Сделать: режим `update` через `/UpdateCfg` Конфигуратора с файлом настроек —
   замер 5.13; единая адресация `--ref`.

7. `SITE-TASK-007` (P2). `check` вместо двух подкоманд `syntax`. Сайт: `check` —
   только `/CheckConfig` со всеми режимами, `/CheckModules` покрыт им (тезис 79);
   для EDT — `validate`. Код: `syntax designer-config` и `syntax designer-modules`
   с 21 и 9 флагами (`src/cli/args.rs:738-816`), `/CheckModules` отдельной веткой
   (`src/use_cases/check_syntax.rs:215`).
   Сделать: одна команда, набор режимов из `/CheckConfig`, `designer-modules`
   уходит в скрытый синоним.

### B. Конфигурация

8. `SITE-TASK-010` (P0). Карта `infobases`, умолчание `origin`.
   Сайт: базы лежат по именам, `origin` берётся без ключа, `init` и `clone` пишут
   `origin`, ключ `infobase` — скрытый синоним `infobases.origin` на один цикл
   (`cli.html` «Начало работы», `#synonyms`). Код: единственный объект `infobase`
   (`src/config/model.rs:39`, `schema.rs:517`); местный слой не принимает `standalone`
   (`schema.rs:615`); прежние ключи отвергаются без синонимов (`loader.rs:315`).
   Сделать: `infobases: map<имя, секция базы>` в основном файле и местном слое,
   секция базы целиком допустима в местном слое, синоним `infobase:` с предупреждением
   один цикл, схема JSON перегенерирована (`CTR.CONFIG.V8PROJECT-SCHEMA`), выбор базы
   по `--infobase` или `origin`, отказ без `origin` называет шаг.
   Приёмка: `tests/config_*` на обе формы; `status --all` перечисляет базы по именам.

9. `SITE-TASK-011` (P0). Строка подключения рядом с SSH-шлюзом у автономной цели.
   Сайт: `connection: Srvr=…;Ref=…` ведёт Конфигуратор в прямой шлюз, `standalone.gate`
   ведёт по SSH; секция `standalone` объявляет вид цели, достаточно любого из двух
   ключей (`architecture.html#targets`, `deployments.html#d-standalone-local`, тезис 65).
   Код: `standalone` вместе с непустой `connection` — `TargetDeclaredTwice`
   (`src/config/validate.rs:63,792`); `exchange` обязателен (`:805`); `dbms` запрещён.
   Сделать: снять взаимное исключение, `exchange` обязателен только когда объявлен
   `gate` и не объявлена строка; `user` и `password` — пользователь базы, по нему же
   пускает шлюз (`connection.rs:37-41` пересмотреть).

10. `SITE-TASK-012` (P1). Секция `cluster` в секции базы: `ras`, `user`, `password`,
    `agent.user`, `agent.password`. Сайт: три уровня учётных данных, каждый
    запрашивается только операцией, которой он нужен (`cli.html` «Сеансы», тезис 59).
    Код: секции нет, серверные реквизиты только в `dbms` (`model.rs:265-285`).
    Нужна задачам D и 020.

11. `SITE-TASK-013` (P1). Ключи `providers.*` по словарю: `push`, `pull`, `apply`,
    `upload`, `download`, `diff`, `make`, `extensions`, `infobase.create|dump|restore`;
    без `providers.check`, `providers.sessions`, `providers.publish` (`cli.html#keys`).
    Код: `init`, `build`, `load`, `dump`, `extensions`, `infobase.configuration.export`,
    `infobase.dump`, `infobase.restore`, `syntax`, `make`, `publish`
    (`src/domain/capability.rs:78-100`, `schema.rs:467-508`).
    Сделать: переименовать перечисление `Operation`, прежние ключи — синонимы по 001,
    `providers.syntax` и `providers.publish` снять (у операции один исполнитель,
    ключ и сегодня отвергается `ProviderKeyWithoutChoice`, `validate.rs:131`).

12. `SITE-TASK-014` (P2). `build.partialLoadThreshold` → `push.partialLoadThreshold`
    (`src/config/model.rs:440`). Синоним по 001.

13. `SITE-TASK-015` (P2). Ключи `tools.*` сверить со словарём сайта: `tools.edt_cli`,
    `tools.designer_agent`, `tools.enterprise.additional-launch-keys`, `tools.client_mcp`
    совпадают; `web` переезжает в секцию базы вместе с остальным (задача 010).

### C. Цели и исполнители

14. `SITE-TASK-020` (P0). `infobase create` по виду цели.
    Сайт: файловую создаёт `ibcmd` сразу с конфигурацией из исходников (`--import`),
    в кластере — Конфигуратор `CREATEINFOBASE` со строкой `Srvr/Ref/DBMS/DBSrvr/DB/
    DBUID/DBPwd/CrSQLDB=Y/SUsr/SPwd/SchJobDn`, `rac` — запасной путь; для автономного
    сервера отказ с рецептом `ibcmd server config init` + `ibcmd infobase create`
    (`cli.html` «Начало работы», тезисы 73–75). Код: `CREATEINFOBASE` только для `File=`
    (`src/platform/designer.rs:178-186`, `connection.rs:114-118`); серверная база —
    `ibcmd infobase create --create-database` по `dbms` (`ibcmd.rs:458-466`,
    `init_project.rs:705-717`); `init` сегодня и есть создание базы (`args.rs:56`).
    Сделать: `CREATEINFOBASE` с клиент-серверной строкой — замер 5.4; `ibcmd` для
    файловой с `--import`; `rac infobase create` запасным путём — замер 5.3; отказ
    для автономной цели с рецептом; заменить `DEC.2026-04-22.INIT-ENSURES-A-SERVER-INFOBASE-THROUGH-IBCMD`.
    Существующая задача TODO 4 (`init --dry-run` для серверной базы) переезжает сюда.

15. `SITE-TASK-021` (P0). Прямой шлюз автономного сервера.
    Сайт: Конфигуратор ходит в автономный сервер как в кластер, первым в цепочке;
    `agent` по SSH — когда строка не объявлена или назначен ключом; `upload`, `check`,
    `diff`, `download --state db`, снимок, `launch designer|thin`, `test` тонким
    клиентом работают по прямому шлюзу; толстый клиент и обычное приложение отказывают
    (`architecture.html#d-ops`, `deployments.html#d-standalone-*`, тезисы 65, 60).
    Код: единственная точка входа — SSH-шлюз (`capability.rs:284-299`,
    `agent_session.rs:166-193`); отказы `load`, `syntax`, `publish`, снимка
    (`provider_selection.rs:56-65`), `test` (`run_tests/coordinator.rs:535-540`),
    `launch designer|thick` (`launch_app.rs:72-79`); `Srvr=` без секции считается
    кластером (`model.rs:322-331`).
    Сделать: строки матрицы для `standalone` с `designer` первым; вид цели по секции,
    строка подключения — канал Конфигуратора; снять отказы; `infobase create` остаётся
    отказом; заменить три инварианта `INV.CLI.A-STANDALONE-*` (раздел G).
    Замеры 5.1 и 5.2 до реализации.

16. `SITE-TASK-022` (P1). Порядок цепочек по виду цели.
    Сайт: файловая база — `agent → designer → ibcmd`, кластер — `agent → designer`,
    `ibcmd` в столбце кластера отсутствует (`architecture.html#d-ops`, тезисы 60, 74).
    Код: `designer → ibcmd`, `agent` экспериментален и в цепочку не входит
    (`capability.rs:233-237,304-310`); `ibcmd` участвует в цепочке серверной базы.
    Сделать: `agent` первым для файловой базы и кластера после стража «агентский
    JSON — структурный вывод» (TODO 7) и подтверждения побайтового равенства выгрузок;
    `ibcmd` из строк кластера убрать (кроме `infobase create` файловой). Убрать
    параллельную матрицу экспорта (`infobase_export.rs:900-957`) — один источник.

17. `SITE-TASK-023` (P1). `make` без базы. Сайт: `ibcmd → ibcmd-rs → designer`, база
    не нужна, Конфигуратор — через временную базу (`architecture.html#d-ops`, тезис 30).
    Код: `ibcmd config import --out` не вызывается, `ibcmd-rs` без адаптера и без строк
    (`provider_selection.rs:45`), `make` идёт Конфигуратором против базы проекта
    (`artifacts.rs:451`). Замер 5.5 для `.cfe`.

18. `SITE-TASK-024` (P1). `launch` по единому правилу адреса.
    Сайт: без ключа тонкий клиент берёт строку подключения, веб-адрес — когда строка
    не объявлена; `--via web|connection` только для тонкого; `launch web` — браузером;
    на автономной цели `thick` и `ordinary` отказывают (`architecture.html`, тезис 67).
    Код: `--via` есть (`launch_app.rs:91-135`), но у автономной цели умолчание — веб, а
    `designer` и `thick` отказывают всегда (`launch_app.rs:72-79`); реквизиты клиенту не
    передаются, потому что считаются реквизитами шлюза.
    Сделать после 011 и 021.

19. `SITE-TASK-025` (P2). Обнаружение платформы по маске версии.
    Сайт: утилиты находятся по маске версии, отказ называет установленные версии
    (`problems.html#p12`). Код: маска — только префикс из 2–4 чисел, без `*` и диапазонов
    (`src/platform/locator.rs:121-155`); при заданном `tools.platform.path` и мягкой
    политике версия игнорируется (`locator.rs:204-212`). Решить, нужна ли маска шире
    префикса; отказ уже называет компонент (`locator.rs:928-990`).

### D. Сеансы и применение

20. `SITE-TASK-030` (P1). `apply` отдельной командой и `push --no-apply`.
    Сайт: `push` отправляет и применяет, `--no-apply` останавливается на основной
    конфигурации, `apply` приводит базу данных и больше ничего (`cli.html`, тезис 3).
    Код: применение — шаг внутри `build` (`build_project.rs:615`), отдельной команды
    нет; `/UpdateDBCfg` только с `-Extension` (`designer.rs:98-109`).
    Сделать: команда `apply`, ключ `--no-apply` у `push`, состояние «есть непринятое»
    в `status`.

21. `SITE-TASK-031` (P1). `apply --sessions disable|force`.
    Сайт: что делать с чужими сеансами, когда нужен монопольный доступ; по умолчанию
    их не трогают (тезисы 4, 5). Код: `-SessionTerminate`, `--session-terminate`
    нигде нет; `ibcmd config apply` всегда `--force --dynamic auto`
    (`ibcmd.rs:378-390`); у агента `update-db-cfg` без ключа
    (`build_project/agent.rs:227-231`). Замер 5.2 для прямого шлюза.

22. `SITE-TASK-032` (P1). `sessions list|terminate|deny|allow` через `rac`.
    Сайт: окно обслуживания собирается из штатных свойств базы, исполнитель один —
    `rac` по `cluster.ras` с администратором кластера, `deny` и `allow` — с
    пользователем базы; у автономного сервера — `ibcmd session`; у файловой базы отказ
    (`cli.html` «Сеансы», `deployments.html#d-ras`, тезисы 56–59). Код: `rac` в `src`
    отсутствует; `ibcmd session` не вызывается.
    Сделать: адаптер `rac` (процесс, разбор вывода, три уровня учётных данных),
    `ibcmd session` по SSH для автономной цели; команды `rac` — из его справки,
    замер 5.3. `providers.sessions` не заводить.

23. `SITE-TASK-033` (P1). `ras`, поднятый раннером.
    Сайт: без `cluster.ras` раннер поднимает `ras cluster` сам, на своей машине, против
    агента кластера из строки подключения, и гасит вместе с командой, как управляемого
    агента Конфигуратора (`deployments.html#d-ras-managed`, тезис 55). Код: `ras`
    не запускается, серверные компоненты в поиске утилит не различаются
    (`src/platform/locator.rs:26-33`).
    Сделать: управляемый `ras` по образцу управляемого агента (`agent_session.rs:221-228`):
    свободный порт, адрес агента из `Srvr=` с портом 1540 или `cluster.agent.address`,
    жизнь с замком, квитанция называет, что сервер администрирования поднят раннером.

### E. Память и первый контакт

24. `SITE-TASK-040` (P0). Память по базе.
    Сайт: под `workPath/infobases/<имя>/` лежат `generation.json`, `hashes/` по
    наборам и `ConfigDumpInfo.xml` этой пары; отправка в `test` не сбивает память о
    `prod` (`sources.html#memory`). Код: хеши по набору исходников в одном `workPath`
    (`src/change_detection/source_set.rs:45-50`, `hash_storage.rs:10-19`), без привязки
    к базе; поколение — только у агента (`agent_session.rs:809-867`);
    `ConfigDumpInfo.xml` лежит в каталоге исходников, исключён из хеширования
    (`scanner.rs:40`), пишется платформой при загрузке (`designer.rs:59-89`).
    Сделать: раскладка памяти по имени базы, файл версий у раннера и подкладывается
    на время команды, полная выгрузка через staging (`INV.USE-CASES.*` перепроверить).

25. `SITE-TASK-041` (P0). Поколение у всех исполнителей и правило первого контакта.
    Сайт: поколение спрашивается до операции и после; `push` в базу, ушедшую вперёд, —
    отказ `non_fast_forward` со ссылкой на `pull`; `push` в непустую базу без памяти —
    отказ с выбором `pull` или `push --force`; сорок нулей — пустая база
    (`cli.html#refusals`, `sources.html`, тезис 13). Код: `/GetConfigGenerationID`
    Конфигуратором не вызывается, `ibcmd config generation-id` — только проба
    существования при создании (`ibcmd.rs:252-266`); агент пишет поколение после
    загрузки и не сравнивает до (`build_project/agent.rs:105-147`).
    Сделать: чтение поколения у трёх исполнителей, сравнение до `push` и `pull`,
    новые виды отказа (задача 005). Замер 5.7.

26. `SITE-TASK-042` (P1). `status`, `status --deep`, `status --all`.
    Сайт: без ключа отвечает по памяти и платформу не запускает; `--deep` спрашивает
    поколение и состав расширений; `--all` перечисляет базы (`sources.html`, `cli.html`).
    Код: команды нет; состав расширений умеет `ibcmd config extension list`
    (`ibcmd.rs:306-309`) и агент, `/DumpDBCfgList` Конфигуратором не вызывается.

27. `SITE-TASK-043` (P1). `pull` как слияние.
    Сайт: `pull` сливает выгрузку с каталогом, при конфликте отказывает и оставляет
    разметку системе версий; `pull --force` заменяет каталог; `pull --all` объявляет
    наборы (`cli.html`, `sources.html#states`). Код: `dump --mode full|incremental|partial`
    заменяет или дописывает каталог, слияния нет (`dump_config.rs`).
    Сделать: трёхстороннее слияние по прошлой выгрузке из памяти; `--all` — замер 5.10.

### F. Исходники и расширения

28. `SITE-TASK-050` (P1). Свойства расширения из исходников.
    Сайт: имя, префикс и назначение — свойства в `Configuration.xml`; `push my-ext`
    заводит расширение в базе сам; `extensions set` правит свойства установленного
    экземпляра (`sources.html`, `cli.html#ext`, тезисы 36, 39). Код: `extensions create`
    требует `--name`, `--name-prefix`, `--purpose` флагами (`src/cli/args.rs:346-357`);
    `push` расширение не заводит; `source_descriptor.rs` уже читает `Name` и `Purpose`.
    Сделать: заведение при `push`, `extensions set` из `activate` и обновления свойств
    (TODO 3 переезжает сюда), `create` и `delete` — скрытые синонимы `push`/`push --delete`.
    Замер 5.8 на переименование.

29. `SITE-TASK-051` (P1). `diff`. Сайт: список изменившихся объектов по файлу версий
    (`/DumpConfigToFiles -getChanges`, `ibcmd export status`), отчёты `/CompareCfg`
    против базы данных, поставщика по имени или пакета (`cli.html`, тезисы 8, 10).
    Код: `-getChanges` не используется, `/CompareCfg` только как проба совместимости
    с фиксированными `-ReportType Brief -ReportFormat txt` (`designer.rs:147-176`).

30. `SITE-TASK-052` (P2). Хранилище конфигурации. Сайт: выгрузка не ограничена, полная
    загрузка недоступна, частичная — для захваченных объектов (тезис 50). Код: команд
    хранилища нет. Замер 5.9, затем решение о захвате перед частичным `push`.

31. `SITE-TASK-053` (P2). `download --state db` у агента. Сайт: у SSH-шлюза только
    основная конфигурация, `--state db` — Конфигуратор (`architecture.html#d-ops`).
    Код совпадает (`infobase_export/agent.rs:33`); переименование по 002.

### G. Реестр решений и контракты

32. `SITE-TASK-060` (P0). Заменить записи, которым целевая модель противоречит, до
    реализации, по `DEC.2026-09-16.A-SUPERSESSION-IS-RECORDED-BY-BOTH-DECISIONS`:
    - `INV.CLI.A-STANDALONE-CLIENT-GOES-BY-THE-WEB-ADDRESS`,
      `INV.CLI.A-STANDALONE-CLIENT-IS-NOT-GIVEN-THE-GATE-CREDENTIALS`,
      `INV.CLI.A-NON-THIN-MODE-STILL-REFUSES-A-STANDALONE-TARGET` — прямой шлюз (021, 024);
    - `DEC.2026-04-22.INIT-ENSURES-A-SERVER-INFOBASE-THROUGH-IBCMD` — создание по виду цели (020);
    - `DEC.2026-04-21.INFOBASE-SECTION-OWNS-CONNECTION-AND-CREDENTIALS` — карта `infobases`,
      секция базы владеет теми же ключами (010);
    - `DEC.2026-09-14.ONLY-A-STANDALONE-SERVER-ANSWERS-WITHOUT-BEING-STARTED` — дополнить:
      отвечает и по прямому шлюзу, раннер его по-прежнему не запускает.
    Оставить в силе: `DEC.2026-09-14.PROVIDER-CHOSEN-PER-OPERATION`,
    `DEC.2026-09-14.THE-TARGET-MAY-LIVE-ON-ANOTHER-MACHINE`,
    `DEC.2026-09-16.A-THIN-CLIENT-OPENS-EITHER-ADDRESS`, `DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE`,
    `INV.PLATFORM.AN-AGENT-FOR-A-FILE-OR-CLUSTER-TARGET-NEEDS-THE-LOCAL-PLATFORM`.

33. `SITE-TASK-061` (P0). Новые решения, которых в реестре нет: имена `upload` и
    `download`; `origin` как умолчание карты `infobases`; скрытые синонимы на один цикл;
    порядок цепочек по виду цели, включая Конфигуратор первым у автономного сервера;
    `infobase create` по виду цели; область `sessions` — только своя база; память по
    базе и правило первого контакта; `pull` как слияние с отказом при конфликте;
    `check` = `/CheckConfig`. Каждое — отдельной записью со ссылкой на страницу сайта.

34. `SITE-TASK-062` (P1). Контракты `CTR.WIRE.*`: переименовать `BOOTSTRAP → CLONE`,
    `BUILD → PUSH`, `DUMP → PULL`, `CONFIG-INIT → INIT`, `SYNTAX → CHECK`,
    `INFOBASE-CONFIGURATION-EXPORT → DOWNLOAD`, `LOAD → UPLOAD`; завести `STATUS`,
    `APPLY`, `DIFF`, `SESSIONS`, `INFOBASE-CREATE`; `CTR.WIRE.COMMAND-ENVELOPE` — поле `next`;
    `CTR.CONFIG.V8PROJECT-SCHEMA` — новая схема с синонимами.
    `CTR.MCP.PUBLISHED-TOOL-SURFACE` не меняется: имена инструментов MCP — свой
    контракт, переименования командной строки в него не протекают (`cli.html#why`).

35. `SITE-TASK-063` (P3). Стражи: тест, что скрытый синоним отсутствует в `--help`;
    тест, что состав инструментов MCP не изменился; проверка, что таблица
    `architecture.html#d-ops` и `docs/site/data.js` (`target()`) совпадают с
    `src/domain/capability.rs`, — скрипт в `scripts/`, запуск в CI; снятие синонимов
    через цикл — отдельной задачей с датой.

## 3. Что уже совпадает

Не требуют работ: `test --no-build` (`src/cli/args.rs:385`), `tools download`
трёх инструментов (`tools_download.rs:106-219`), `mcp serve stdio|http`,
`launch mcp` с `--wait-ready`, `publish` через `webinst` с превью и маскированием,
`convert` между EDT и XML, `download --state db` Конфигуратором
(`infobase_export.rs:1470-1471`), `extensions list` через `ibcmd` и агента, агентский
режим managed/attached с одной сессией на команду, канал обмена `sftp|dir` у SSH-шлюза,
замок `workPath`, маскирование секретов, отсутствие срока у команды, отказ
`ws=…` в строке подключения, ключи расширений `installedName|namePrefix|purpose` в
конфиге уже отсутствуют.

## 4. Существующие задачи TODO, которые поглощаются

- TODO 1 (preflight монопольного доступа для `ibcmd` DT) — остаётся, относится к 022.
- TODO 2 (принудительное завершение сеансов при `infobase restore`) — решается 031 и 032.
- TODO 3 (`extensions … update` до полного набора свойств) — переезжает в 050.
- TODO 4 (`init --dry-run` для серверной базы) — переезжает в 020.
- TODO 7 (агентский провайдер, шаг 2) — страж структурного вывода нужен 022.
- TODO 10 и 13 (автономный сервер, шаг 3; цель на другой машине) — пересматриваются 021.

## 5. Замеры

Каждый замер — запись в `spec/acceptance/` или `references/1c/` с датой, версией
платформы и командной строкой.

1. Порт прямого шлюза `ibsrv` и строка `Srvr=host[:port];Ref=<имя>` для Конфигуратора
   и тонкого клиента; имя равно `--name` сервера (тезис 65).
2. Пакетные команды Конфигуратора по прямому шлюзу: `/LoadConfigFromFiles`,
   `/DumpConfigToFiles`, `/UpdateDBCfg -SessionTerminate`, `/CheckConfig`,
   `/CompareCfg`, `/DumpDBCfg`, `/DumpIB`, `/RestoreIB`.
3. Команды `rac` для `sessions list|terminate|deny|allow` и `rac infobase create`:
   состав ключей из `rac help` (тезис 58), поведение при заполненных списках
   администраторов (тезис 59).
4. `CREATEINFOBASE` с клиент-серверной строкой: обязательные поля, `CrSQLDB=Y`,
   `SUsr`/`SPwd`, `SchJobDn`, код выхода при существующей базе (тезис 73).
5. `ibcmd config import --out` для `.cf` и `.cfe` без базы (тезис 30).
6. Состав `ibcmd config check` (тезис 105).
7. `/GetConfigGenerationID` с `/Out` в пакетном режиме и `ibcmd config generation-id`
   на живой базе СУБД (тезис 13; TODO 4).
8. Переименование расширения через `push`: что делает платформа, когда имя в
   `Configuration.xml` изменилось (тезисы 36, 39).
9. Частичный `push` в базу под хранилищем: нужен ли захват объектов (тезис 50).
10. `pull --all`: объявление наборов по составу базы.
11. `ibcmd session list|terminate` по SSH к автономному серверу.
12. `dump-ib` через SSH-шлюз роняет `ibsrv` 8.3.27 (замер 15.09.2026) — проверить на
    следующей сборке платформы; снимок по прямому шлюзу — замер 2.
13. `/UpdateCfg` с файлом настроек для `upload --mode update` и `/MergeCfg` для
    `combine` (тезис 49).

## 6. Порядок работ

1. Реестр: задачи 060–062, ревью скептиком до кода (`AGENTS.md`, Review Gate).
2. Схема конфигурации: 010, 011, 012, 013, 014; схема JSON и `docs/CONFIGURATION.md`.
3. Словарь: 001, 002, 004, 005, 007; `docs/CAPABILITIES.md` перегенерировать из матрицы.
4. Память и первый контакт: 040, 041, 042, 043.
5. Цели: 020, 021, 022, 024; замеры 1, 2, 4 перед 020 и 021.
6. Операции: 030, 031, 032, 006, 050, 051, 023; замеры 3, 5, 13.
7. Стражи 063 и снятие синонимов через цикл.

Сайт при этом не правится под текущее состояние: он остаётся целью, а расхождения
живут здесь до закрытия.
