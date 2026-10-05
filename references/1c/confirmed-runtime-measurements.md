# Замеры на живой платформе

Факты из этого файла получены запуском настоящих утилит 1С, а не чтением документации.
Без установленной платформы они не восстанавливаются, поэтому лежат здесь и пережили
удаление прежнего слоя решений, где были записаны впервые.

Файл описывает поведение платформы, а не обязательства раннера: гарантии живут правилами
в [`spec/rules/`](../../spec/rules/README.md).

Везде, где не сказано иначе, платформа — `8.3.27.2074`.

## Загрузка информационной базы из DT

Замер на файловой базе (выброшенные копии).

| | `ibcmd infobase restore` | Конфигуратор `/RestoreIB` |
| --- | --- | --- |
| цель существует | заменяет, `rc=0`, без вопросов | заменяет, `rc=0`, без вопросов |
| цель отсутствует | `rc=255`, «Отсутствует файл базы данных»; каталог и `1Cv8.cgr.cfl` остаются | создаёт базу, `rc=0` |
| цель отсутствует, есть `--create-database` | создаёт и загружает, `rc=0` | ключа нет, создаёт всегда |
| вход не читается | `rc=255`, «Файл не обнаружен» | не мерено |
| завершение сеансов | `--force`, `--session-terminate-message` | ключа нет |

Шлюза у замены нет ни у одного исполнителя: опечатка в пути стирает данные без вопроса.
Создание ведёт себя по-разному: Конфигуратор создаёт всегда, `ibcmd` без
`--create-database` отказывает — один и тот же вызов даёт разный исход в зависимости от
исполнителя.

## Язык прозы платформы

Замер 12.09.2026. Задача: [#86](https://github.com/IngvarConsulting/v8-runner-rust/issues/86).

Один и тот же факт платформа пишет тремя способами:

- `Конфигурация 'Конфигурация поставщика' недоступна` — по умолчанию установки;
- `Configuration Vendor configuration is not available` — при `/L en`;
- `Konfiguration 'Lieferantenkonfiguration' ist nicht verfügbar` — при `/L de`.

Закрепить язык не помогает. `/L` — ключ Конфигуратора; у `ibcmd` управления языком нет
(ключа не нашли, локаль он игнорирует). Английский при этом доступен всегда: отдельного
пакета `1cv8_en.res` нет, английский несёт `1cv8_root.res`, и любой неизвестный код
языка (`/L ja`, `/L pt`) отвечает по-английски.

Структурные ответы у платформы есть: код выхода `/CompareCfg` равен нулю тогда и только
тогда, когда сравнение состоялось, и ровно тогда появляется файл отчёта; `ibcmd config
extension list` отвечает ключами и значениями на английском (`active: yes`,
`purpose: patch`), от языка не зависящими; `ibcmd config generation-id` возвращает
идентификатор поколения.

Это замеренное основание правила о том, что решение никогда не принимается по прозе
инструмента.

## Агентский режим и автономный сервер

Замер 13.09.2026. Задача: [#82](https://github.com/IngvarConsulting/v8-runner-rust/issues/82).

**Агентский режим Конфигуратора и SSH-шлюз автономного сервера — один и тот же shell.**
Баннеры и приглашения разные (`designer>` и `user@DefAlias>`), а `help config` совпадает
побайтно: `dump-cfg`, `dump-config-to-files`, `load-cfg`, `load-config-from-files`,
`generation-id`, `extensions …`, `manage-cfg-support`, `sign-cfg`, `update-db-cfg`,
`dump-ib`, `restore-ib`. Отдельный процесс Конфигуратора для автономного сервера не нужен.

**Покрытие у этого shell'а своё.** В нём нет `check-config`, `CompareCfg`, `MergeCfg` —
они остались у пакетного Конфигуратора. Ответы приходят массивом JSON с закрытым
множеством `error-type`, чего у пакетного режима нет.

**Аутентификация подчиняется правилу пакетного Конфигуратора.** База без пользователей:
агент принимает пустой логин с пустым паролем, шлюз — любое имя с пустым паролем (замер
06.10.2026 на 8.3.27.2074 это уточняет: шлюз принял пустой пароль только с пустым именем, см.
«Сеансы автономного сервера через SSH-шлюз»). База с
пользователями: только пользователь ИБ и его пароль. Пользователя «по умолчанию» нет.

**Готовность зависит от среды, а не от конфига.** `ibcmd --pid` отвечает через 10–20 с
после старта автономного сервера; в одной из замеренных конфигураций — с
`--enable-extended-designer-features` — не ответил за десять минут. `ibcmd --pid config
export` пишет ноль файлов и не завершается. `ibcmd --remote=ssh://` работает только с
терминалом — уточнено 06.10.2026: терминал нужен, только пока ключа хоста нет в
`~/.ssh/known_hosts`; без терминала и без ключа `ibcmd` падает с SIGSEGV, с известным ключом
работает без терминала.

**Раскладка файлов у агента своя.** Все пути команд агент разрешает относительно
`<AgentBaseDir>/<каталог пользователя>`; соответствие «пользователь → каталог» он сам
пишет в `<AgentBaseDir>/agentbasedir.json` (пустой логин → `0`). Абсолютный путь
трактуется как относительный и укладывается под тот же каталог; `..` выходит на уровень
`AgentBaseDir`, но не выше. Шлюз `ibsrv` делает то же относительно каталога
`--users-data`, а в сообщении называет файл `local:///gate.cf`. Выгрузка
`dump-config-to-files` побайтно совпадает с `ibcmd infobase config export` той же базы
(128 файлов, `diff -rq` пуст).

**Ключ хоста.** При `/AgentSSHHostKeyAuto` агент публикует ключ из
`~/.1cv8/1C/1cv8/host_id` — путь замерен только на Linux, на сборке 8.3.27.1859. Строки
замера — в [`designer-agent/request-surface.md`](designer-agent/request-surface.md).

**Внешний клиент SSH не годится.** Замеренный рецепт через `SSH_ASKPASS_REQUIRE=force`
требует OpenSSH ≥ 8.4, чего на Windows не гарантировано, а пустое имя пользователя
внешний клиент принимает только через `-l ''`. Поэтому клиент SSH у раннера свой,
в процессе (`russh`).

**Исполнитель без платформы существует и в умолчания не годится.** `ibcmd-rs` (Павел
Чегодаев) конвертирует XML↔CF без Конфигуратора с байтовой точностью 98,9 % против
родной выгрузки.

## Идентификатор поколения

Замер 14.09.2026. Идентификатор поколения отдают все три исполнителя:
`/GetConfigGenerationID` у пакетного Конфигуратора, `ibcmd config generation-id` и
агентская сессия. По сессии ответ приходит примерно за секунду, отдельным процессом — за
четыре и больше.

Токен основной конфигурации у Конфигуратора и `ibcmd` совпал побайтно. **Токены одного и
того же расширения у них различаются**, поэтому сравнение значений от разных инструментов
ничего не доказывает.

Прежняя запись о провайдерах утверждала, что у пакетного Конфигуратора `generation-id` нет;
этот замер её опровергает.

## Идентификатор поколения у базы СУБД в кластере

Замер 06.10.2026. Задача: [#184](https://github.com/IngvarConsulting/v8-runner-rust/issues/184). Платформа 8.5.4.1878 (кластер в Docker linux/amd64 под эмуляцией на macOS, PostgreSQL 17.10; стенд — в разделе «Кластер в Docker: стенд»).

**Конфигуратор не замерен — нет подходящей лицензии.** `1cv8 DESIGNER /S 'onec.localhost:1541\v8rm_x2' /DisableStartupDialogs /DisableStartupMessages /Out <файл> /GetConfigGenerationID`:

- с Mac — rc=1, 12 с, `/Out` (UTF-8 с BOM, CRLF): `Операция не может быть выполнена с текущим составом лицензий.` /
  `Сервер 1С:Предприятия использует лицензию для разработчиков. Запуск клиентского приложения Конфигуратор с лицензией ПРОФ или КОРП запрещён. …`;
- из контейнера `srv` (`xvfb-run`, пользователь `onec`) — rc=1, ~20 с, `/Out`: `Не найдена лицензия. Не обнаружен ключ защиты программы или полученная программная лицензия!`;
- `license-distribution=allow` у базы не помог ни там, ни там. Linux-клиент `client` с лицензией разработчика всё
  время замера был в состоянии `Restarting (255)`. Поэтому формат `/Out` при успехе, совпадение с `ibcmd` и
  поведение после `/LoadConfigFromFiles` и `/UpdateDBCfg` у Конфигуратора в кластере не получены.

**`ibcmd`** — внутри `srv` (на Mac PostgreSQL недоступен), пароль СУБД через `-W` со stdin.

| Шаг на базе `v8rm_g` | rc | stdout | Время |
| --- | --- | --- | --- |
| пустая база (`rac infobase create --create-database`) | 0 | `2af84151e959af78eab1cb38d137eedf33543af5` | ~20 с |
| то же значение у пустых баз из `rac` без `--create-database` и из `CREATEINFOBASE` в готовую БД | 0 | `2af84151e959af78eab1cb38d137eedf33543af5` | |
| `config import /exchange/v8rm/cfg` (загрузка без применения) | 0 | `[INFO] Импорт конфигурации из XML...` / `…успешно завершен` | ~20 с |
| после загрузки | 0 | `29579ceccaf9ef9b04b0e500bc90ab1aec7370ab` — изменился | |
| два чтения подряд без изменений | 0 | одно и то же значение | |
| повторный `config import` тех же файлов | 0 | `5035e5fb13d070a05bfc4b18832e11436c660219` — **снова изменился** | |
| `config apply --force` (и с `--dynamic=disable`) | 139 | `[INFO] Обновление конфигурации базы данных...` / `…Проверка корректности метаданных...` / `…Обработка структуры базы данных...`; stderr `qemu: uncaught target signal 11 (Segmentation fault) - core dumped` | ~13 с |
| после упавшего `apply` | 0 | `5035e5…` — без изменений | |
| `--extension=NoSuchExt` | 255 | stderr `Расширение конфигурации не найдено.` | |
| БД без ИБ (брошенная после сбоя) | 255 | stderr `[WARN] Поколение информационной базы не найдено: db:head` и та же строка без `[WARN]` | |
| БД нет | 255 | stderr `Соединение с сервером баз данных разорвано администратором` / `…база данных "v8rm_none" не существует` | 4 с |

Формат ответа: в stdout сначала приглашение `-W` (`Введите пароль для подключения к базе данных: ` и перевод
строки), затем 40 строчных hex-символов и `\n`. В stderr при каждом обращении к PostgreSQL — три строки
`ПРЕДУПРЕЖДЕНИЕ:  нет незавершённой транзакции` (уведомления сервера СУБД), в том числе при rc=0.
**Значение пустой базы зависит от версии платформы, а не от вида базы.** Пустая база в кластере 8.5.4
отдаёт постоянное `2af84151e959af78eab1cb38d137eedf33543af5`. Контроль на файловых базах того же дня
(`ibcmd infobase create`, затем `ibcmd infobase config generation-id`, обе `rc=0`): 8.3.27.2074 — сорок
нулей, 8.5.4.1878 — то же `2af84151…`.
Поведение после применения не замерено: `apply` под эмуляцией QEMU падает дважды подряд.

```sh
# внутри srv; пароль СУБД — со stdin:
docker compose exec -T db cat /run/secrets/postgres_password | docker compose exec -T -u onec srv \
  ibcmd config generation-id --dbms=PostgreSQL --db-server=db --db-name=v8rm_g --db-user=postgres -W
… ibcmd config import /exchange/v8rm/cfg --dbms=PostgreSQL --db-server=db --db-name=v8rm_g --db-user=postgres -W
… ibcmd config apply --force [--dynamic=disable] --dbms=PostgreSQL --db-server=db --db-name=v8rm_g --db-user=postgres -W
# Конфигуратор (отказ по лицензии):
/opt/1cv8/8.5.4.1878/1cv8 DESIGNER /S 'onec.localhost:1541\v8rm_x2' /DisableStartupDialogs /DisableStartupMessages \
  /Out <файл> /GetConfigGenerationID
docker compose exec -T -u onec srv xvfb-run -a /opt/1cv8/x86_64/8.5.4.1878/1cv8 DESIGNER /S 'onec.localhost:1541\v8rm_x2' \
  /GetConfigGenerationID /DisableStartupDialogs /DisableStartupMessages /Out /exchange/v8rm/g1.out
```

**Вывод для потребителей.** Чтение поколения (#215): у `ibcmd` значение — последняя непустая строка stdout,
40 hex; приглашение `-W` и предупреждения PostgreSQL в stderr ответом не являются и при rc=0. Читать до и после
операции тем же инструментом. Токен меняется уже от загрузки без применения и от повторной загрузки тех же файлов,
поэтому сравнение токенов (#167) отвечает только на вопрос «трогали ли основную конфигурацию с прошлого чтения», а
не «совпадает ли содержимое»: равенство — изменений не было, неравенство — была запись, возможно того же самого.
«Сорок нулей» как признак пустой базы держится только на 8.3.27: на 8.5.4 пустая база — и файловая, и в
кластере — отдаёт `2af84151e959af78eab1cb38d137eedf33543af5`.
Вопросы о Конфигураторе в кластере и о смене токена после применения остаются открытыми до стенда с лицензией
разработчика у клиента и без эмуляции.

## Список изменений выгрузки: `-getChanges`

Замер 06.10.2026. Задача: [#173](https://github.com/IngvarConsulting/v8-runner-rust/issues/173). Платформа 8.3.27.2074, macOS.

Файловая база, созданная `ibcmd infobase create` и заполненная фикстурой
`tests/fixtures/designer/configuration` (`/LoadConfigFromFiles` + `/UpdateDBCfg`). Каждый вызов
ниже — отдельный процесс Конфигуратора, 2,4–3,5 с; `<W>` — рабочий каталог замера. Префикс
всех вызовов Конфигуратора:

```text
1cv8 DESIGNER /F <W>/ib /DisableStartupDialogs /DisableStartupMessages <команда> /Out <W>/out/<метка>.log
```

**Главное: `-getChanges` ничего не выгружает.** С этим ключом команда только вычисляет список и
пишет файл; каталог выгрузки остаётся побайтно прежним, у файлов не меняется даже время
изменения (`find -newermt` после вызова — 0 файлов), `ConfigDumpInfo.xml` тоже не трогается. Это
проверено и с `-update`, и без него, и в случае `FullDump`. Чтобы получить и список, и выгрузку,
нужны два вызова подряд: сначала `-getChanges`, затем `-update`.

**Формат файла.** UTF-8 с BOM, строки через CRLF, одна запись на строку, в конце CRLF. В записи —
вид изменения и полное имя объекта метаданных, не путь к файлу. Сначала `New:`, затем `Modified:`
по возрастанию имени. Нет изменений — файл из одного BOM (3 байта). Наблюдали ровно три вида
записей: `New: <объект>`, `Modified: <объект>` и одиночную строку `FullDump`. Записи `Deleted`
не было ни разу: удаление объекта или реквизита давало `FullDump`. Исчерпывающим это множество
не считаем: оно собрано по сценариям ниже, а не по документации.

```text
00000000: efbb bf4d 6f64 6966 6965 643a 2043 6f6d  ...Modified: Com
00000010: 6d6f 6e4d 6f64 756c 652e d09e d0b1 d189  monModule.......
00000020: d0b8 d0b9 d09c d0be d0b4 d183 d0bb d18c  ................
00000030: 310d 0a4d 6f64 6966 6965 643a 2043 6f6d  1..Modified: Com
00000040: 6d6f 6e4d 6f64 756c 652e d09e d0b1 d189  monModule.......
00000050: d0b8 d0b9 d09c d0be d0b4 d183 d0bb d18c  ................
00000060: 312e 4d6f 6475 6c65 0d0a                 1.Module..

00000000: efbb bf46 756c 6c44 756d 700d 0a         ...FullDump..
```

Имена — те же, что в атрибуте `name` у `ConfigDumpInfo.xml`: `Catalog.Справочник3` соответствует
`Catalogs/Справочник3.xml`, `CommonModule.ОбщийМодуль1.Module` —
`CommonModules/ОбщийМодуль1/Ext/Module.bsl`, `Role.Администратор.Rights` —
`Roles/Администратор/Ext/Rights.xml`, `Configuration.Конфигурация` — `Configuration.xml`.
Для путей имена придётся отображать самим.

### Сценарии: список против диска

Каждый сценарий: правка в базе, затем `-update -getChanges` по копии прежней выгрузки, затем
обычный `-update` по другой копии. Файлы копии заранее помечались старой датой, `diff -rq` шёл
против выгрузки «до».

| Правка в базе | `-getChanges` | `-update`: перезаписано файлов (mtime) | `-update`: изменилось по содержимому |
| --- | --- | --- | --- |
| нет | пустой (BOM) | — | — |
| модуль, частичная загрузка `Module.bsl` | `Modified:` модуль и `.Module` | 2 + `ConfigDumpInfo.xml` (`ОбщийМодуль1.xml`, `Module.bsl`) | `Module.bsl`, `ConfigDumpInfo.xml` |
| свойство справочника, частичная загрузка его XML | `Modified: Catalog.Справочник3` | 1 + `ConfigDumpInfo.xml` | эти же 2 файла |
| новый справочник, частичная загрузка `Configuration.xml` и его XML | `New: Catalog.Справочник3` и `Modified:` для всех 50 объектов | все 52 | новый файл, `Configuration.xml`, `Module.bsl` (правка предыдущего шага), `ConfigDumpInfo.xml` |
| удалён объект `Бот1`, частичная загрузка `Configuration.xml` | `FullDump` | все | `Configuration.xml`, `ConfigDumpInfo.xml`, удалён каталог `Bots/` |
| удалён реквизит регистра, частичная загрузка XML регистра | `FullDump` | все 51 | XML регистра, `ConfigDumpInfo.xml` |
| полная загрузка того же дерева без изменений | `Modified:` для всех 50 объектов, `New` нет | — | побайтно ничего |
| полная загрузка: правка модуля и свойства, новый объект, удалён `Отчет1` | `FullDump` | все 51 | 4 файла, `ConfigDumpInfo.xml`, удалён `Reports/` |

Что из этого следует:

- Список описывает **версии** объектов (`configVersion` в `ConfigDumpInfo.xml`), а не содержимое
  файлов. Полная загрузка или добавление объекта через `Configuration.xml` обновляют версию у всех
  объектов. Тогда весь список — `Modified`, `-update` переписывает все файлы, а по содержимому
  меняется 0–4 файла. Для объектов без `FullDump` множество перезаписанных `-update` файлов
  совпало со списком.
- `FullDump` — платформа переходит на полную выгрузку. При удалении реквизита изменилась версия
  одного объекта (`diff` по `ConfigDumpInfo.xml`: одна строка версии, одна удалённая строка), а
  `-update` переписал все 51 файл. Вызов с `-getChanges` заранее говорит, что выгрузка будет
  полной.
- `-update` удаляет файлы удалённых объектов (каталоги `Bots/`, `Reports/`), но посторонние файлы
  оставляет (`stray.txt`, `Catalogs/stray.xml` пережили и обычный `-update`, и `FullDump`).
  Полная выгрузка **без** `-update` в существующий каталог не удаляет ничего: `Bots/Бот1.xml`
  остался, хотя объекта в конфигурации уже нет.

### Вопросы задачи

| Вопрос | Ответ |
| --- | --- |
| (1) `-getChanges` с `-update`, без `-configDumpInfoForChanges` | работает, `rc=0`, `/Out` пуст (BOM и перевод строки) |
| `-getChanges` без `-update`, в каталоге есть `ConfigDumpInfo.xml` | работает так же: тот же список, `rc=0`, каталог не трогается |
| (3) `-getChanges` без `-update` в несуществующий каталог | `rc=1`, «Не удалось найти файл версий - …/ConfigDumpInfo.xml.», файла нет |
| (3) `-update -getChanges`: каталога нет, каталог пуст или в нём нет `ConfigDumpInfo.xml` | `rc=1`, та же фраза, файла нет |
| (3) `-update -getChanges`: `ConfigDumpInfo.xml` испорчен | `rc=1`, «Ошибка разбора XML: … Document is empty», файла нет |
| (3) `-update -getChanges`: в `ConfigDumpInfo.xml` `version="2.17"` | принят, `rc=0`, обычный список |
| (3) `-update [-force] -getChanges`: в `ConfigDumpInfo.xml` `version="9.99"` | `rc=1`, «Неизвестная версия формата 9.99 …»; `-force` не помогает |
| обычный `-update` (без `-getChanges`) в тех же случаях: нет каталога, нет файла, файл испорчен (и с `-force`) | `rc=1`, те же сообщения; молча на полную выгрузку платформа не переходит |
| (4) совпадение со списком на диске | см. таблицу сценариев: список — надмножество изменений содержимого, объекты, а не пути; при `FullDump` перечня нет вовсе |

Платформа сама переходит на полную выгрузку только в одном замеренном случае: при `-update` по
целому `ConfigDumpInfo.xml`, когда в конфигурации что-то удалено. Об этом сообщает `FullDump`.
Отсутствующий или испорченный `ConfigDumpInfo.xml` полной выгрузкой не заканчивается: `-update`
отказывает с `rc=1`.

### `-configDumpInfoForChanges <файл>`

| Вызов | Результат |
| --- | --- |
| `-update -getChanges <ch> -configDumpInfoForChanges <cdi>` в непустой каталог | `rc=1`, «Каталог … не пуст.» |
| то же в несуществующий или пустой каталог | `rc=0`, список считается против `<cdi>`, а не против каталога; каталог не создаётся |
| `-getChanges <ch> -configDumpInfoForChanges <cdi>` без `-update`, каталога нет | `rc=0`, тот же список |
| старый `<cdi>`, после которого был удалён реквизит | `FullDump` |
| `-update -configDumpInfoForChanges <cdi>` **без** `-getChanges`, каталога нет | `rc=0`, **выгружает только изменённое**: `ConfigDumpInfo.xml` (полный, новый) + `CommonModules/ОбщийМодуль1.xml` + `.../Ext/Module.bsl` |
| то же со старым `<cdi>` (случай `FullDump`) | `rc=0`, полная выгрузка, 51 файл |
| `-configDumpInfoForChanges` без `-update` и `-getChanges` | `rc=1`, «Использование ключа -configDumpInfoForChanges возможно только совместно с ключами -update и/или -getChanges» |

Справочник (`DumpConfigToFiles.md`) пишет, что `-configDumpInfoForChanges` используется только
вместе с `-update` **и** `-getChanges`. Платформа принимает «и/или», а с одним `-update` даёт
выгрузку дельты в пустой каталог.

### `ibcmd config export status` (найдено попутно)

`ibcmd help config` в 8.3.27 описывает `config export status --base=<ConfigDumpInfo>
[--short] [--out=<file>] [--extension=<name>]`: изменения базы относительно сохранённого файла
версий. Тот же вид ответа, только в своём словаре:

```text
ibcmd config export status --db-path=<W>/ib --base=<W>/infos/cdi_d8.xml --out=<W>/st.txt          → rc=0
  ﻿modified: CommonModule.ОбщийМодуль1
  modified: CommonModule.ОбщийМодуль1.Module
ibcmd config export status --db-path=<W>/ib --base=<W>/infos/cdi_d8.xml --short --out=<W>/st.txt  → rc=0
  ﻿M: CommonModule.ОбщийМодуль1
  M: CommonModule.ОбщийМодуль1.Module
ibcmd config export status ... --base=<W>/infos/cdi_d7.xml          → ﻿modified: all
ibcmd config export status ... --base=<W>/infos/cdi_d7.xml --short  → ﻿M: all
```

Тоже UTF-8 с BOM и CRLF. Случай, в котором Конфигуратор пишет `FullDump`, у `ibcmd` выглядит как
`modified: all` / `M: all`. Расширение без изменений против своего `ConfigDumpInfo.xml` — `rc=0`,
пустой вывод. Время — около 5 с на вызов.

### Командные строки

```text
/LoadConfigFromFiles <W>/src_cfg /UpdateDBCfg
/DumpConfigToFiles <W>/d0
/DumpConfigToFiles <W>/d_fullgc -getChanges <W>/ch.txt                                 # каталога нет → rc=1
/DumpConfigToFiles <W>/dA -getChanges <W>/chA.txt                                      # каталог с ConfigDumpInfo → rc=0
/DumpConfigToFiles <W>/dB -update -getChanges <W>/chB.txt
/LoadConfigFromFiles <W>/src3 -files "CommonModules/ОбщийМодуль1/Ext/Module.bsl" -partial /UpdateDBCfg
/LoadConfigFromFiles <W>/src3 -files "Configuration.xml,Catalogs/Справочник3.xml" -partial /UpdateDBCfg
/LoadConfigFromFiles <W>/src3 -files "Configuration.xml" -partial /UpdateDBCfg          # удаление Бот1
/LoadConfigFromFiles <W>/src3 -files "AccumulationRegisters/РегистрНакопления1.xml" -partial /UpdateDBCfg
/DumpConfigToFiles <W>/dX -update -getChanges <W>/chX.txt
/DumpConfigToFiles <W>/dX -update
/DumpConfigToFiles <W>/dX -update -force -getChanges <W>/chX.txt
/DumpConfigToFiles <W>/dU -update -getChanges <W>/ch.txt -configDumpInfoForChanges <W>/infos/cdi.xml
/DumpConfigToFiles <W>/dV3 -update -configDumpInfoForChanges <W>/infos/cdi.xml
```

### Вывод для потребителей

- **`diff` (#219).** Годится как дешёвый вопрос «что изменилось в базе относительно этой
  выгрузки», без выгрузки и без записи в дерево: один вызов около 3 с, каталог не трогается. Но
  ответ — объекты метаданных, а не файлы: для показа путей нужно отображение имён в пути. Ответ
  описывает версии, а не содержимое: после полной загрузки или добавления объекта через
  `Configuration.xml` все объекты значатся `Modified`, хотя по содержимому изменилось 0–4 файла.
  Удалений список не называет совсем: вместо них приходит `FullDump`. Тот же ответ без
  Конфигуратора даёт `ibcmd config export status --base`.
- **Режим выгрузки (#166).** Узнать можно, но только **заранее** и отдельным вызовом: `FullDump`
  в ответе `-getChanges` (у `ibcmd` — `modified: all`) значит, что `-update` по этому
  `ConfigDumpInfo.xml` будет полным. После выгрузки ключ ничего не сообщает, потому что в одном
  вызове с выгрузкой он не работает. Между двумя вызовами база может измениться. Случаев «нет
  или испорчен `ConfigDumpInfo.xml`» признак не покрывает: и `-getChanges`, и `-update` в них
  дают `rc=1`, а не полную выгрузку. Вопрос о признаке возвращается владельцу. Признак есть,
  но это прогноз отдельным вызовом, а не отчёт самой выгрузки.
- **Память после выгрузки (#53).** Напрямую нет. Список нужно получить до выгрузки, и он
  перечисляет перезаписанные объекты, а не изменённые файлы (при массовом `Modified`
  перезаписаны все файлы). При `FullDump` список пуст по смыслу. Удаления не названы, а
  посторонние файлы `-update` оставляет. Надёжнее сверять `ConfigDumpInfo.xml` до и после
  выгрузки: изменившиеся `configVersion` и пропавшие строки дают ровно перезаписанные и
  удалённые объекты. Это сравнение двух файлов, без лишнего вызова платформы.
  `-update -configDumpInfoForChanges` в пустой каталог даёт только изменившиеся файлы, а это
  готовая дельта — если раннеру нужна дельта, а не отчёт.

## Список расширений и выгрузка каждого

Замер 06.10.2026. Задача: [#187](https://github.com/IngvarConsulting/v8-runner-rust/issues/187). Платформа 8.3.27.2074, macOS.

Новая файловая база с фикстурой `configuration`. В неё загружены два расширения: `Расширение1`
— фикстура `tests/fixtures/designer/extension`; `Расширение2` — её копия, в которой заменены
`Name`, префикс `Расш1_` → `Расш2_` и все uuid, кроме `xr:ClassId` и
`ExtendedConfigurationObject`. С новыми `ClassId` загрузка отказывает:
«Неверный идентификатор класса хранимого объекта», `rc=1`. Затем `Расширение2` выключено
через `ibcmd config extension update --active=no`.

### Кто как отдаёт список

| Исполнитель | Вызов | Что отдаёт | Выключенное расширение | Время |
| --- | --- | --- | --- | --- |
| пакетный Конфигуратор | `/DumpDBCfgList -AllExtensions` | только имена, по одному на строку, в `/Out` (UTF-8 с BOM) | в списке, признака активности нет | 2,9 с |
| пакетный Конфигуратор | `/DumpDBCfgList -Extension Расширение2` | `Расширение2`, `rc=0`; несуществующее имя — `rc=1`, «…расширение конфигурации с указанным именем не найдено: НетТакого» | — | 2,9 с |
| пакетный Конфигуратор | `/DumpDBCfgList` без ключей | `rc=1`, «Ошибка в параметрах командной строки.» | — | 2,8 с |
| `ibcmd` | `ibcmd config extension list --db-path=<ib>` (и синоним `ibcmd extension list`) | блоки «ключ : значение» на stdout: `name : "Расширение1"`, `active`, `purpose`, `safe-mode`, `scope`, `hash-sum` и др. | `active : no` | 5,4 с |
| `ibcmd` | `ibcmd config extension info --name=<имя>` | тот же блок для одного расширения | — | 5,3 с |
| агент Конфигуратора | `config extensions properties get --all-extensions` | JSON: `type: "success"`, в `body` объекты `type: "extension-properties"` с `name`, `active` (bool), `purpose`, `hash-sum`… | `"active": false` | доли секунды в сессии |
| агент Конфигуратора | `config extensions properties get --extension=Расширение2` | JSON-массив из одного `extension-properties` **без** обёртки `success` | — | — |

В базе без расширений `/DumpDBCfgList -AllExtensions` даёт `rc=0` и пустой `/Out`. Команды
`list` у агента нет: список отдаёт `properties get --all-extensions`. `hash-sum` у `ibcmd` и у
агента совпал побайтно.

### Выгрузка каждого расширения

| Вызов | Результат |
| --- | --- |
| `/DumpConfigToFiles <W>/ext_all -AllExtensions` | `rc=0`, 3,1 с; `ext_all/Расширение1/`, `ext_all/Расширение2/` — в каждом свой `Configuration.xml` и `ConfigDumpInfo.xml`; выключенное выгружено тоже |
| `ibcmd config export all-extensions --db-path=<ib> <W>/ext_all_ibcmd` | `rc=0`, 5,7 с; та же раскладка, `diff -rq` с выгрузкой Конфигуратора пуст |
| агент: `config dump-config-to-files --dir=ext_agent --all-extensions` | та же раскладка под `<AgentBaseDir>/0/ext_agent`, `diff -rq` с `ext_all` пуст |
| `/DumpConfigToFiles <W>/ext_one -Extension Расширение2` | `rc=0`; файлы расширения прямо в `ext_one/`, совпадают с `ext_all/Расширение2/` |
| `/DumpConfigToFiles <W>/main_only` (без ключей расширений) | только основная конфигурация, расширений в выгрузке нет |
| `/DumpConfigToFiles <W>/ext_all -AllExtensions -update [-getChanges …]` | `rc=1`, «Не удалось найти файл версий - <W>/ext_all/ConfigDumpInfo.xml.» — файл версий ищется в корне, а не в подкаталогах |
| `/DumpConfigToFiles <W>/ext_one -Extension Расширение2 -update -getChanges <ch>` | `rc=0`, пустой список (BOM) |

Имя подкаталога равно `Name` расширения. Признака активности в выгрузке нет: это свойство базы,
а не конфигурации расширения.

```text
1cv8 DESIGNER /F <W>/ib2 /DisableStartupDialogs /DisableStartupMessages /LoadConfigFromFiles <W>/src_ext1 -Extension Расширение1 /UpdateDBCfg /Out …
1cv8 DESIGNER /F <W>/ib2 … /DumpDBCfgList -AllExtensions /Out …
1cv8 DESIGNER /F <W>/ib2 … /DumpDBCfgList -Extension Расширение2 /Out …
1cv8 DESIGNER /F <W>/ib2 … /DumpConfigToFiles <W>/ext_all -AllExtensions /Out …
1cv8 DESIGNER /F <W>/ib2 … /DumpConfigToFiles <W>/ext_one -Extension Расширение2 /Out …
ibcmd config extension list --db-path=<W>/ib2
ibcmd config extension update --db-path=<W>/ib2 --name=Расширение2 --active=no
ibcmd config export all-extensions --db-path=<W>/ib2 <W>/ext_all_ibcmd
1cv8 DESIGNER /F <W>/ib2 /AgentMode /AgentListenAddress 127.0.0.1 /AgentPort 15731 /AgentBaseDir <W>/agentbase /AgentSSHHostKeyAuto
ssh -T -l '' -p 15731 127.0.0.1   # SSH_ASKPASS с пустым паролем; команды: options set --output-format=json; common connect-ib; config extensions properties get --all-extensions; config dump-config-to-files --dir=ext_agent --all-extensions
```

Агент поднялся за ~4 с; ответ пришёл после `connect-ib`; остановлен командой `common shutdown`.

### Вывод для потребителей

- **`pull --all` (#344).** Список с `Name` отдают все три исполнителя: `/DumpDBCfgList -AllExtensions`,
  `ibcmd config extension list` и агент `config extensions properties get --all-extensions`.
  Активность видна только у `ibcmd` и агента; у пакетного Конфигуратора — одни имена.
  Каждое расширение выгружается в свой каталог с `Configuration.xml` одним вызовом
  `-AllExtensions` (`ibcmd config export all-extensions`, агент `--all-extensions`). Имя
  подкаталога — `Name`, раскладка у трёх исполнителей побайтно одинакова. Выключенное расширение
  выгружается наравне с включённым. Инкрементальной выгрузки по `-AllExtensions` нет:
  `-update` ищет `ConfigDumpInfo.xml` в корне и отказывает с `rc=1`. Для `-update` каждое
  расширение выгружается отдельным вызовом `-Extension <Name>` в свой каталог.
- **«Расширение без проекта» в `status --deep` (#216).** Множество имён из любого из трёх
  вызовов сравнивается с `Name` расширений, объявленных в проекте. Пакетному Конфигуратору для
  этого хватает `/DumpDBCfgList -AllExtensions` (одно имя на строку, без прозы). Если в строке
  нужна активность, источник — `ibcmd config extension list` (поле `active: yes|no`) или агент.

## Сборка пакета из XML через `ibcmd config import --out`

Замер 06.10.2026. Задача: [#182](https://github.com/IngvarConsulting/v8-runner-rust/issues/182). Платформа 8.3.27.2074, macOS.

Вход — фикстуры `tests/fixtures/designer/configuration` (51 файл) и
`tests/fixtures/designer/extension` (расширение `Расширение1`). `$W` — рабочий каталог замера.

**Имя команды и ключа.** В 8.3.27 команда — `ibcmd config import`, ключ — `--out=<file> | -o <file>`
(«Путь к файлу для записи импортируемой конфигурации»), рядом `--extension=<name> | -e`.
Аргумент — каталог или архив с XML.

**Без базы команда не работает.** Ей нужна существующая файловая база; ни один ключ её не
заменяет:

| вызов | rc | ответ |
| --- | --- | --- |
| без `--db-path`, без `--data` | 255 | «Отсутствует файл базы данных '~/.1cv82/1C/1Cv82/standalone-server/db-data/1Cv8.1CD'» |
| `--db-path=<пустой каталог>` | 255 | «Отсутствует файл базы данных '<каталог>/1Cv8.1CD'»; в каталоге остаётся `1Cv8.cgr.cfl` |
| `--db-path=<несуществующий каталог>` | 255 | то же; каталог **создаётся** с `1Cv8.cgr.cfl` внутри |
| `--data=<пустой каталог>` | 255 | «Отсутствует файл базы данных '<data>/db-data/1Cv8.1CD'»; в `<data>` появляются `db-data ipc-data perf-data temp users-data` |
| `--db-path=<пустая база из ibcmd infobase create>` | 0 | пакет записан |

**С `--out` база не меняется.** На пустой базе `sha1` файла `1Cv8.1CD` до и после импорта
совпал, `config generation-id` остался нулевым. Годится и база с уже загруженной
конфигурацией — пакет собирается из XML, а не из базы. **Без `--out` та же команда
загружает XML в базу** (`rc=0`, `generation-id` стал ненулевым): опечатка в ключе
превращает сборку в загрузку.

**Расширение собирается тем же вызовом.** Тип пакета берётся из XML, а не из ключа:

| вход | `--extension` | rc | размер | содержимое после загрузки в базу |
| --- | --- | --- | --- | --- |
| конфигурация | нет | 0 | 116 076 | конфигурация |
| конфигурация | `X` | 0 | 116 080 | та же конфигурация (`diff -rq` отличается только `configVersion` в `ConfigDumpInfo.xml`) |
| расширение | `Расширение1` | 0 | 6 824 | расширение `Расширение1` |
| расширение | нет | 0 | 6 824 | то же, `diff -rq` пуст |
| расширение | `Другое` | 0 | 6 824 | то же, имя внутри — `Расширение1`, `diff -rq` пуст |

**Пакет не детерминирован.** Два импорта одного XML дают разные байты (`cmp -l`: 36 775
различий у `.cf`, 3 271 у `.cfe`), размер плавает на единицы байт. Поэтому сравнение с
Конфигуратором — только через выгрузку.

**С Конфигуратором совпадает по содержимому, не по байтам.**

| пакет | `ibcmd config import --out` | Конфигуратор `/LoadConfigFromFiles` + `/DumpCfg` |
| --- | --- | --- |
| `.cf` | 116 076 байт | 114 995 байт |
| `.cfe` | 6 824 байт | 6 164 байт |

Каждый пакет загружен `/LoadCfg` (расширение — `/LoadCfg … -Extension Расширение1` в базу
с конфигурацией) в чистую базу и выгружен `/DumpConfigToFiles`. Выгрузки `.cf`:
`diff -rq` отличает только `ConfigDumpInfo.xml`, и после удаления атрибутов
`configVersion` он совпадает; остальные 50 файлов равны. Выгрузки `.cfe`: `diff -rq` пуст.

**Коды выхода.**

| случай | rc | текст |
| --- | --- | --- |
| успех | 0 | `[INFO] Импорт конфигурации из XML успешно завершен` |
| нет базы | 255 | «Отсутствует файл базы данных …» |
| каталог для `--out` не существует | 255 | `[ERROR] … Файл не обнаружен '<out>'. 2(0x00000002): No such file or directory`; файла нет |
| входного каталога нет | 255 | `[ERROR] … Файл не обнаружен '<path>'` |
| битый `Configuration.xml` | 255 | `[ERROR] … Ошибка разбора XML: … Document is empty`; файла нет |
| `--out` на существующий файл | 0 | файл перезаписан без вопроса |
| вход — ZIP с XML | 0 | пакет собран |
| занят каталог данных сервера | 254 | «Ошибка блокировки каталога данных сервера. Рабочий каталог заблокирован процессом: N» |
| занята база (`--data` разные, `--db-path` один) | 255 | «Ошибка исключительной блокировки информационной базы.» |

**Блокировка — на каталоге данных, а не на базе.** Два параллельных `config import` с
умолчательным `--data` конфликтуют (`rc=254`), даже если `--db-path` разные. С разными
`--data` и разными `--db-path` оба проходят. С разными `--data` и одной базой второй
получает `rc=255`. Время: создание базы `ibcmd infobase create` ~5 с, импорт ~5 с
(`.cfe` — 5–8 с).

```text
ibcmd config import --out=$W/out182/a1.cf $W/xml_cf                                   # rc=255
ibcmd config import --db-path=$W/e_empty --out=$W/out182/a2.cf $W/xml_cf              # rc=255
ibcmd config import --db-path=$W/e_none --out=$W/out182/a3.cf $W/xml_cf               # rc=255
ibcmd config import --data=$W/d_data --out=$W/out182/a4.cf $W/xml_cf                  # rc=255
ibcmd infobase create --db-path=$W/ib_tmp                                             # rc=0
ibcmd config import --db-path=$W/ib_tmp --out=$W/out182/b1.cf $W/xml_cf               # rc=0
ibcmd config import --db-path=$W/ib_tmp --extension=Расширение1 --out=$W/out182/c1.cfe $W/xml_cfe  # rc=0
ibcmd config import --db-path=$W/ib_tmp --out=$W/out182/c2.cfe $W/xml_cfe             # rc=0
ibcmd config import --db-path=$W/ib_tmp --out=$W/out182/g7.cf $W/xml_cf.zip           # rc=0
ibcmd config import --db-path=$W/ib_tmp $W/xml_cf                                     # rc=0, загрузил в базу
1cv8 DESIGNER /F $W/ib_d /DisableStartupDialogs /LoadConfigFromFiles $W/xml_cf /Out …  # rc=0
1cv8 DESIGNER /F $W/ib_d /DisableStartupDialogs /DumpCfg $W/d182/designer.cf /Out …     # rc=0
1cv8 DESIGNER /F $W/ib_d /DisableStartupDialogs /LoadConfigFromFiles $W/xml_cfe -Extension Расширение1 /Out …  # rc=0
1cv8 DESIGNER /F $W/ib_d /DisableStartupDialogs /DumpCfg $W/d182/designer.cfe -Extension Расширение1 /Out …     # rc=0
1cv8 DESIGNER /F $W/x_ib /DisableStartupDialogs /LoadCfg $W/out182/b1.cf /Out …        # rc=0
1cv8 DESIGNER /F $W/x_ib /DisableStartupDialogs /DumpConfigToFiles $W/dump_cf_ib /Out …  # rc=0
```

**Вывод для потребителей.** `ibcmd config import --out` собирает и `.cf`, и `.cfe` с тем же
содержимым, что Конфигуратор, без Конфигуратора и без изменения базы, но **без базы не
работает**: ему нужна существующая файловая база, хоть пустая. Значит, в `make` без базы
(#207) и в `convert` XML → пакет (#236) `ibcmd` входит только с выброшенной базой,
которую раннер создаёт сам (`ibcmd infobase create`, ~5 с) в своём временном каталоге,
со своим `--data`, чтобы не упереться в блокировку каталога данных у параллельных
вызовов. `--extension` для типа пакета не нужен и имя в пакет не попадает. Вызов
обязан всегда нести `--out`: без него та же команда пишет в базу. Пакет не
детерминирован, поэтому кэш или «не изменилось» по байтам пакета невозможны. Этот пакет
— обычный `.cf`, не файл поставки: `/UpdateCfg` его не принимает (см. следующий раздел).

## Обновление конфигурации на поддержке через `/UpdateCfg`

Замер 06.10.2026. Задача: [#190](https://github.com/IngvarConsulting/v8-runner-rust/issues/190). Платформа 8.3.27.2074, macOS.

**Подготовка.** `v1` — фикстура `configuration` с `Vendor=ТестПоставщик`,
`Version=1.0.0.1`. `v2` — то же, `Version=1.0.0.2`, плюс функция `ВерсияПоставщика` в
`ОбщийМодуль1` и реквизит `РеквизитВ2` у `Справочник1`. Пакеты:

- `v1.cf`, `v2.cf` — обычные, `ibcmd config import --out`;
- `v2_dumpcfg.cf` — обычный, `/DumpCfg` Конфигуратора;
- `v1_dist.cf`, `v2_dist.cf`, `v2.cfu` — файлы поставки, `/CreateDistributionFiles` из баз
  с `v1`/`v2` (`-cfufile v2.cfu -f v1_dist.cf`). Команда требует обновлённой конфигурации
  БД: без `/UpdateDBCfg` — `rc=1`, «Для создания файлов требуется обновить конфигурацию
  базы данных».

Признак поддержки — файл `Ext/ParentConfigurations.bin` в выгрузке `/DumpConfigToFiles`.
Состояние конфигурации БД — `/DumpDBCfg`, загрузка в чистую базу и выгрузка в XML.

**Как поставить базу на поддержку.**

| способ | rc | поддержка |
| --- | --- | --- |
| `/LoadCfg v1.cf` (обычный) в пустую базу | 0 | нет |
| `/LoadCfg v1_dist.cf` (поставка) в пустую базу | 0 | **да**, правила «не редактируется» |
| `/MergeCfg v1.cf -Settings … -EnableSupport` (обычный; пустая база или база с `v1`; любые `ConfigurationsRelation` и `SupportRules`) | 1 | «Возможность объединения с постановкой на поддержку отсутствует» |
| `/MergeCfg v1_dist.cf -Settings … -EnableSupport` (поставка; пустая база или база с `v1`) | 0 | **да**, правила из `SupportRules` файла настроек |

Поддержку даёт только файл поставки; тип пакета решает, правила — нет.

**Формат `-Settings`.** Схема лежит в `frntend_root.res` платформы: пространство
`http://v8.1c.ru/8.3/config/merge/settings`, корень `Settings` с обязательным атрибутом
`version`, внутри `Parameters` (`ConfigurationsRelation`,
`AllowMainConfigurationObjectDeletion`, …), `SupportRules`, `Conformities`, `Objects`
(`Configuration`/`Object` с `MergeRule` из `DoNotMerge`, `GetFromSecondConfiguration`,
`MergePrioritizingMainConfiguration`, `MergePrioritizingSecondConfiguration`,
`MergeWithExternalTool`). Замер шёл на файле:

```xml
<Settings xmlns="http://v8.1c.ru/8.3/config/merge/settings" version="1.2" platformVersion="8.3.27">
	<Parameters>
		<ConfigurationsRelation>SecondConfigurationIsDescendantOfMainConfiguration</ConfigurationsRelation>
		<AllowMainConfigurationObjectDeletion>true</AllowMainConfigurationObjectDeletion>
	</Parameters>
	<Objects>
		<Configuration><MergeRule>GetFromSecondConfiguration</MergeRule></Configuration>
	</Objects>
</Settings>
```

Раннер этот файл не формирует: для `/MergeCfg` он передаёт путь, данный пользователем
(`upload --mode combine --settings`).

**Что принимает `/UpdateCfg`.** Время каждого вызова ~3 с.

| база | файл | `-Settings` | rc | текст `/Out` | основная конфигурация |
| --- | --- | --- | --- | --- | --- |
| без поддержки | `v2.cf` | нет / есть | 1 | «Файл не содержит доступных обновлений» | без изменений |
| без поддержки | `v2_dist.cf` | есть | 1 | то же | без изменений |
| без поддержки | `v2.cfu` | есть | 1 | то же | без изменений |
| на поддержке | `v2.cf` (ibcmd) | есть | 1 | то же | без изменений |
| на поддержке | `v2_dumpcfg.cf` (Конфигуратор) | есть | 1 | то же | без изменений |
| на поддержке | `v2_dist.cf` | нет | 0 | «Обновление конфигурации успешно завершено» | `1.0.0.2` |
| на поддержке | `v2_dist.cf` | есть | 0 | то же | `1.0.0.2` |
| на поддержке | `v2.cfu` | нет / есть | 0 | то же | `1.0.0.2` |
| база с расширением | `.cfe` (`v2` расширения) | есть, с `-Extension` и без | 1 | «Файл не содержит доступных обновлений» | без изменений |

`/UpdateCfg` обновляет только конфигурацию на поддержке и только файлом поставки или
обновления. Обычный `.cf` — тот, что собирает `ibcmd config import` или `/DumpCfg`, —
не принимается даже на поддержке. Для `.cfe` команда не работает; то же расширение
`/MergeCfg ext2.cfe -Settings … -Extension Расширение1` обновляет (`rc=0`, версия
расширения стала `2.0`).

**`-Settings` читается только при конфликте.** Если в базе нет собственных изменений
(правила «не редактируется»), файл настроек не читается вовсе: несуществующий путь,
файл `garbage` и `MergeRule=DoNotMerge` — все `rc=0` и обновление до `1.0.0.2`. Локальная
правка модуля в такой базе (через `/LoadConfigFromFiles`) при обновлении молча теряется,
с любыми настройками.

База на поддержке с правилами «редактируется с сохранением поддержки» (`/MergeCfg
v1_dist.cf -EnableSupport`) и локальной процедурой в `ОбщийМодуль1`:

| `-Settings` | rc | текст `/Out` | версия | функция поставщика | локальная процедура |
| --- | --- | --- | --- | --- | --- |
| нет | 1 | «Невозможно выполнение обновления конфигурации в командном режиме» | 1.0.0.1 | нет | есть |
| `GetFromSecondConfiguration` | 0 | «Обновление конфигурации успешно завершено» | 1.0.0.2 | есть | **потеряна** |
| `MergePrioritizingMainConfiguration` | 0 | то же | 1.0.0.2 | есть | есть |
| `DoNotMerge` | 0 | то же | **1.0.0.1** | нет | есть |
| `garbage` | 1 | «Ошибка разбора XML: … Document is empty …» + «Невозможно выполнение обновления конфигурации в командном режиме» | 1.0.0.1 | нет | есть |
| несуществующий файл | 1 | «Файл не обнаружен '<путь>'. 2(0x00000002) …» + то же | 1.0.0.1 | нет | есть |

При `DoNotMerge` — `rc=0` и «успешно завершено», а конфигурация не изменилась: код выхода
не говорит, что обновление применилось.

**После `/UpdateCfg` нужен `/UpdateDBCfg`.** Во всех успешных случаях конфигурация БД
(`/DumpDBCfg`) осталась `1.0.0.1` без реквизита и функции. Отдельный `/UpdateDBCfg`
следом — `rc=0`, «Объект изменен: Справочник.Справочник1», конфигурация БД стала
`1.0.0.2`. В одном запуске `/UpdateCfg … -Settings … /UpdateDBCfg` — `rc=0`, в `/Out` оба
сообщения, конфигурация БД `1.0.0.2`. Своего ключа `-UpdateDBCfg` у `/UpdateCfg` в
справке нет. `-force`, `-IncludeObjectsByUnresolvedRefs`, `-ClearUnresolvedRefs`,
`-DumpListOfTwiceChangedProperties` не мерились.

```text
1cv8 DESIGNER /F $W/C /DisableStartupDialogs /LoadCfg $W/u190/v1_dist.cf /Out …                       # rc=0, на поддержке
1cv8 DESIGNER /F $W/E /DisableStartupDialogs /MergeCfg $W/u190/v1_dist.cf -Settings $W/u190/esE_SecondConfigurationIsDescendantOfMainConfiguration.xml -EnableSupport /Out …  # rc=0
1cv8 DESIGNER /F $W/S /DisableStartupDialogs /MergeCfg $W/u190/v1.cf -Settings $W/u190/settings_enable_support.xml -EnableSupport /Out …  # rc=1
1cv8 DESIGNER /F $W/V2 /DisableStartupDialogs /CreateDistributionFiles -cffile $W/u190/v2_dist.cf -cfufile $W/u190/v2.cfu -f $W/u190/v1_dist.cf /Out …  # rc=0
1cv8 DESIGNER /F $W/AA /DisableStartupDialogs /UpdateCfg $W/u190/v2_dist.cf -Settings $W/u190/settings.xml /Out …  # rc=1, без поддержки
1cv8 DESIGNER /F $W/U3 /DisableStartupDialogs /UpdateCfg $W/u190/v2.cf -Settings $W/u190/settings.xml /Out …       # rc=1, обычный cf
1cv8 DESIGNER /F $W/U1 /DisableStartupDialogs /UpdateCfg $W/u190/v2_dist.cf /Out …                                   # rc=0
1cv8 DESIGNER /F $W/U4 /DisableStartupDialogs /UpdateCfg $W/u190/v2.cfu -Settings $W/u190/settings.xml /Out …       # rc=0
1cv8 DESIGNER /F $W/EU1 /DisableStartupDialogs /UpdateCfg $W/u190/v2_dist.cf /Out …                                  # rc=1, конфликт
1cv8 DESIGNER /F $W/EU5 /DisableStartupDialogs /UpdateCfg $W/u190/v2_dist.cf -Settings $W/u190/settings_merge_main.xml /Out …  # rc=0
1cv8 DESIGNER /F $W/U10 /DisableStartupDialogs /UpdateCfg $W/u190/v2_dist.cf -Settings $W/u190/settings.xml /UpdateDBCfg /Out …  # rc=0
1cv8 DESIGNER /F $W/X /DisableStartupDialogs /UpdateCfg $W/u190/ext2.cfe -Settings $W/u190/settings.xml -Extension Расширение1 /Out …  # rc=1
1cv8 DESIGNER /F $W/X /DisableStartupDialogs /MergeCfg $W/u190/ext2.cfe -Settings $W/u190/settings.xml -Extension Расширение1 /Out …    # rc=0
```

**Вывод для потребителей (#196).** `upload --mode update` — это
`/UpdateCfg <файл> -Settings <файл>` и только для основной конфигурации, стоящей на
поддержке; входом годится файл поставки `.cf` (`/CreateDistributionFiles -cffile`) или
обновления `.cfu`, а не `.cf` из `make`/`ibcmd config import`/`/DumpCfg`. Для `.cfe`
режим `update` отказывать до запуска платформы: путь расширения — `/MergeCfg … -Extension`.
`-Settings` делать обязательным: без него обновление базы с собственными изменениями
отказывает (`rc=1`), а при отсутствии конфликтов файл не читается и вреда не приносит;
битый или отсутствующий файл обнаруживается платформой только при конфликте, поэтому
существование и разбор файла раннер проверяет сам. `rc=0` не доказывает, что
обновление применилось (`DoNotMerge`), — подтверждать по версии конфигурации или
идентификатору поколения до и после. Отказ «Файл не содержит доступных обновлений»
(`rc=1`) означает и «база не на поддержке», и «файл не поставка» — различить их по коду
нельзя. После `/UpdateCfg` конфигурация БД не обновлена: нужен `/UpdateDBCfg`, его можно
дописать в тот же запуск Конфигуратора.

## Прямой шлюз автономного сервера

Замер 06.10.2026. Задача: [#178](https://github.com/IngvarConsulting/v8-runner-rust/issues/178). Платформа 8.3.27.2074, macOS.

**Прямой шлюз включён по умолчанию.** `ibsrv` без `--enable-direct-gate` слушает основной
порт прямого соединения и первые порты диапазона; выключает его `--disable-direct-gate`.
Основной порт задаёт `--direct-regport` (по умолчанию 1541), диапазон —
`--direct-range` (по умолчанию `1560:1591`). Ключа адреса у прямого шлюза нет: при
`--direct-regport=18341 --direct-range=18360:18391` сервер слушает `*:18341`, `*:18360`,
`*:18361` на всех интерфейсах (IPv4 и IPv6), хотя HTTP (`--http-address`, по умолчанию
`localhost`) и SSH (`--ssh-address`, по умолчанию `127.0.0.1`) слушают только локальный
адрес. Клиент приходит на основной порт, а данные идут через порт диапазона
(`[::1]:…->[::1]:18360` у тонкого клиента).

**SSH-шлюз тоже включён по умолчанию**, и без ключа хоста сервер не стартует:
`[FATAL] Приватный ключ хоста не найден или поврежден: ~/.ssh/id_rsa`. Ключ задаёт
`--ssh-host-key` (замер: RSA 2048 в PEM).

**Конфигуратор принимает обе формы строки.** `/S "host:port\имя"` и
`/IBConnectionString "Srvr=host:port;Ref=имя;"` дают один результат. Без порта
Конфигуратор идёт на 1541 — порт по умолчанию у клиента совпадает с `--direct-regport`.
Порт из диапазона вместо основного не годится.

**`Ref` — это `--name`, без учёта регистра; идентификатор базы не принимается.**
`Ref=TESTIB` при `--name=testib` подключается. `Ref=<uuid базы>` при заданном `--name`
отвечает «не обнаружена». Без `--name` имя базы — `DefAlias` (и `defalias`), а не
«строковое представление идентификатора», как пишет `ibsrv --help`: при
`--id=11111111-2222-3333-4444-555555555555` строки `Ref=<этот uuid>`, `Ref={<uuid>}`,
`Ref=default` отвечают «не обнаружена». Тем же именем называется приглашение SSH-шлюза
(`@DefAlias>`; при `--name=testib` — `@testib>`). При заданном `--name` имя `DefAlias` не
принимается.

| Строка подключения Конфигуратора | Код | Время | Текст `/Out` |
| --- | --- | --- | --- |
| `/S "127.0.0.1:18341\testib"` | 0 | 4,1 с | (пусто у `/DumpConfigToFiles`; 51 файл, как у исходника) |
| `/IBConnectionString "Srvr=127.0.0.1:18341;Ref=testib;"` | 0 | 4,2 с | `Сохранение конфигурации успешно завершено` |
| `/S "localhost:18341\testib"` | 0 | 4,3 с | то же |
| `Srvr=127.0.0.1:18341;Ref=TESTIB;` | 0 | 4,4 с | то же |
| `/S "127.0.0.1:18341\wrongname"` (неверное имя) | 1 | 4,1 с | `Информационная база не обнаружена!` |
| `Srvr=127.0.0.1:18341;Ref=<uuid базы>;` при `--name=testib` | 1 | 4,5 с | `Информационная база не обнаружена!` |
| `Srvr=127.0.0.1:18341;Ref=;` | 1 | — | `Неопределена информационная база` |
| `/S "127.0.0.1:18342\testib"` (неверный порт, никто не слушает) | 1 | 33,9 с | `Ошибка при выполнении операции с информационной базой` / `server_addr=tcp://127.0.0.1:18342 descr=127.0.0.1:18342:61(0x0000003D): Connection refused;` / `line=1066 file=src/rtrsrvc/src/DataExchangeTcpClientImpl.cpp` |
| `/S "127.0.0.1\testib"` (без порта, на 1541 никого) | 1 | 33,8 с | то же с `tcp://127.0.0.1:1541` |
| `Srvr=127.0.0.1:18360;Ref=testib;` (порт диапазона) | 1 | 4,3 с | `Сервер 1С:Предприятия не обнаружен` / `Адрес 'tcp://127.0.0.1:18360' не является адресом кластера серверов 1С:Предприятия` |
| сервер без `--name`: `Ref=DefAlias` / `Ref=defalias` | 0 | ~4 с | `Сохранение конфигурации успешно завершено` |

Отказ по недоступному порту приходит только через ~34 с: Конфигуратор повторяет попытки.
Порт по умолчанию доказывает строка с `tcp://127.0.0.1:1541` выше: собственный сервер на
1541 в замере не поднимался.

**Тонкий клиент подключается по обеим формам, но `Ref` не проверяет.** `1cv8c ENTERPRISE`
с `/S "127.0.0.1:18341\testib"` и с `/IBConnectionString "Srvr=127.0.0.1:18341;Ref=testib;"`
открывает сеанс (`app-id: 1CV8C` в `ibcmd session list`) через ~20 с и остаётся в GUI —
процесс снимался по таймауту (код 142, SIGALRM), `/Out` пуст. **С неверным именем
(`/S "127.0.0.1:18341\wrongname"`, `Ref=wrongname`) тонкий клиент тоже открыл сеанс в той
же базе** (`infobase: ebc9cce7-…`, единственная база сервера): там, где Конфигуратор
отказывает, тонкий клиент подключается. С неверным портом тонкий клиент сам завершается
кодом 1 через 30 с, в `/Out` — та же строка `server_addr=tcp://127.0.0.1:18342 …
Connection refused;`. Сеанс убитого клиента остаётся в списке сервера, пока его не
завершат (`ibcmd session terminate`).

```sh
P=/opt/1cv8/8.3.27.2074
$P/ibcmd infobase create --data=$B/ibcmd-data --db-path=$B/ib1 --import=$B/src_cfg --apply --force
ssh-keygen -t rsa -b 2048 -m PEM -N "" -f $B/hostkey
$P/ibsrv --data=$B/srv1 --db-path=$B/ib1 --name=testib --http-port=18314 \
  --direct-regport=18341 --direct-range=18360:18391 --ssh-port=18343 --ssh-host-key=$B/hostkey
$P/ibsrv --data=$B/srv2 --db-path=$B/ib2 --id=11111111-2222-3333-4444-555555555555 --http-port=18315 \
  --direct-regport=18351 --direct-range=18392:18399 --ssh-port=18353 --ssh-host-key=$B/hostkey
lsof -nP -a -p <pid ibsrv> -iTCP -sTCP:LISTEN
$P/1cv8 DESIGNER /S "127.0.0.1:18341\testib" /DisableStartupDialogs /DisableStartupMessages /DumpConfigToFiles $B/dump1 /Out $B/out.txt
$P/1cv8 DESIGNER /IBConnectionString "Srvr=127.0.0.1:18341;Ref=testib;" /DisableStartupDialogs /DisableStartupMessages /DumpDBCfg $B/d5.cf /Out $B/out.txt
$P/1cv8c ENTERPRISE /S "127.0.0.1:18341\testib" /DisableStartupDialogs /DisableStartupMessages /Out $B/c1.out
$P/1cv8c ENTERPRISE /IBConnectionString "Srvr=127.0.0.1:18341;Ref=wrongname;" /DisableStartupDialogs /DisableStartupMessages /Out $B/c6.out
```

**Вывод для потребителей.** Для #205: строка прямого шлюза —
`Srvr=<host>:<--direct-regport>;Ref=<--name>` (или `/S "<host>:<port>\<имя>"`); порт по
умолчанию 1541 и у сервера, и у клиента, поэтому без порта строка указывает на 1541;
порт диапазона в строку не ставится. `Ref` — имя `--name` без учёта регистра; при
сервере без `--name` — `DefAlias`, не идентификатор базы. Ошибку имени даёт только
Конфигуратор (код 1, `Информационная база не обнаружена!`, ~4 с); тонкий клиент имя не
проверяет, поэтому проверять цель тонким клиентом нельзя. Неверный порт — код 1 и
`Connection refused` в `/Out`, но через ~34 с. Для
`INV.CONFIG.A-STANDALONE-TARGET-ACCEPTS-EITHER-GATE-KEY`: оба шлюза включены по
умолчанию и работают одновременно над одной базой (SSH-шлюз и прямой в этом замере
обслуживали одну и ту же базу); строка прямого шлюза однозначно выводится из
`--direct-regport` и `--name`. Отдельно: прямой шлюз слушает все интерфейсы, и
ограничить его адресом ключами `ibsrv` нельзя.

## Пакетный Конфигуратор через прямой шлюз

Замер 06.10.2026. Задача: [#179](https://github.com/IngvarConsulting/v8-runner-rust/issues/179). Платформа 8.3.27.2074, macOS.

Все восемь команд выполняются через прямой шлюз `ibsrv` (оговорка про `/RestoreIB` — в таблице): каждая
занимает ~4 с (запуск Конфигуратора), `ibsrv` после каждой жив. Строка подключения во
всех прогонах — `/S "127.0.0.1:18341\testib" /DisableStartupDialogs
/DisableStartupMessages`. Для проверки исключительной блокировки держался открытый сеанс
тонкого клиента (`1cv8c ENTERPRISE /S "127.0.0.1:18341\testib"`).

| Команда | Код | Время | Результат | Текст `/Out` |
| --- | --- | --- | --- | --- |
| `/DumpConfigToFiles <dir>` | 0 | 4,1 с | 51 файл, столько же, сколько в исходной фикстуре; после загрузки — побайтно равен загруженному каталогу, кроме `ConfigDumpInfo.xml` и концов строк в модуле | пусто |
| `/LoadConfigFromFiles <dir>` | 0 | 5,3 с | основная конфигурация изменена (`DescriptionLength` 25→50, строка в модуле) — подтверждено выгрузкой и `/CompareCfg` | пусто |
| `/CompareCfg -FirstConfigurationType MainConfiguration -SecondConfigurationType DBConfiguration -ReportType Full -IncludeChangedObjects -ReportFormat txt -ReportFile <f>` | 0 | 4,3 с | отчёт (UTF-16LE) называет `ОбщийМодуль1` и `Справочник1`, «Длина наименования» 50/25; после `/UpdateDBCfg` — только легенда | пусто |
| то же с `-ReportType Brief` без `-Include…` | 0 | 4,3 с | отчёт содержит только легенду, хотя различия есть | пусто |
| `/CheckConfig -ConfigLogIntegrity -IncorrectReferences -ThinClient` | 0 | 4,3 с | — | `Ошибок не обнаружено` |
| `/UpdateDBCfg` (без `-SessionTerminate`, открыт сеанс тонкого клиента, нужна реструктуризация) | 1 | 4,3 с | база не изменена, сеанс жив | `Обработка структуры базы данных...` / `Ошибка исключительной блокировки информационной базы.` / `Активные сеансы и соединения:` / `компьютер: <имя>, сеанс начат: 06.10.2026 в 0:48:09, приложение: Тонкий клиент` |
| `/UpdateDBCfg -SessionTerminate force` (тот же сеанс открыт) | 0 | 9,5 с | сеанс тонкого клиента завершён (список сеансов пуст, процесс клиента остался жив), конфигурация БД обновлена | `Обработка структуры базы данных...` / `Сбор служебной информации...` / `Объект изменен: Справочник.Справочник1` / `Принятие изменений...` / `Построение индекса справки...` / `Обновление конфигурации успешно завершено` |
| `/LoadConfigFromFiles <dir> /UpdateDBCfg` (без сеансов) | 0 | 7,3 с | загрузка и обновление одним запуском | как выше |
| `/DumpDBCfg <file.cf>` | 0 | 4,2 с | `.cf` 114 976 байт | `Сохранение конфигурации успешно завершено` |
| `/DumpIB <file.dt>` | 0 | 4,3 с | `.dt` 70 550 байт; `ibsrv` не упал (в отличие от `infobase-tools dump-ib` через SSH-шлюз, задача [#189](https://github.com/IngvarConsulting/v8-runner-rust/issues/189)) | `Выгрузка информационной базы успешно завершена` |
| `/RestoreIB <file.dt>` при открытом сеансе | 1 | 4,4 с | база не изменена | `Ошибка исключительной блокировки информационной базы.` / `Активные сеансы и соединения:` / `компьютер: …, сеанс начат: 06.10.2026 в 0:52:09, приложение: Тонкий клиент` |
| `/RestoreIB <file.dt> -SessionTerminate force` при открытом сеансе | 1 | 4,4 с | ключ не действует: тот же отказ, сеанс жив | то же |
| `/RestoreIB <file.dt>` после `ibcmd session terminate --remote=ssh://…` | 0 | 7,4 с | база вернулась к снимку: `DescriptionLength` 50 вместо загруженного 100, `/DumpDBCfg` побайтно равен снятому до изменения; `ibsrv` жив | `Загрузка информационной базы успешно завершена` |

```sh
P=/opt/1cv8/8.3.27.2074; C=(/S "127.0.0.1:18341\testib" /DisableStartupDialogs /DisableStartupMessages)
$P/1cv8 DESIGNER $C /LoadConfigFromFiles $B/mod_a /Out $B/e1.out
$P/1cv8 DESIGNER $C /CompareCfg -FirstConfigurationType MainConfiguration -SecondConfigurationType DBConfiguration \
  -ReportType Full -IncludeChangedObjects -ReportFormat txt -ReportFile $B/e2b_report.txt /Out $B/e2b.out
$P/1cv8 DESIGNER $C /CheckConfig -ConfigLogIntegrity -IncorrectReferences -ThinClient /Out $B/e3.out
$P/1cv8 DESIGNER $C /UpdateDBCfg /Out $B/e5.out
$P/1cv8 DESIGNER $C /UpdateDBCfg -SessionTerminate force /Out $B/e6.out
$P/1cv8 DESIGNER $C /DumpDBCfg $B/e8.cf /Out $B/e8.out
$P/1cv8 DESIGNER $C /DumpIB $B/e9.dt /Out $B/e9.out
$P/1cv8 DESIGNER $C /RestoreIB $B/e9.dt /Out $B/e11.out
$P/1cv8 DESIGNER $C /RestoreIB $B/e9.dt -SessionTerminate force /Out $B/e12.out
$P/ibcmd session list --pid=<pid ibsrv>
```

**Вывод для потребителей.** Для #205: через прямой шлюз Конфигуратору можно ставить все
восемь операций, и первыми — те, которых у SSH-шлюза нет или которые он делает плохо:
`/CompareCfg` (проба совместимости для `load`), `/CheckConfig` (`syntax`), `/DumpIB` и
`/RestoreIB` (снимок; через SSH-шлюз `dump-ib` ронял `ibsrv`, здесь — нет). Для #211:
`/UpdateDBCfg -SessionTerminate force` через прямой шлюз работает — завершает чужие сеансы
и обновляет базу, без ключа отказывает кодом 1 с текстом об исключительной блокировке.
У `/RestoreIB` ключа `-SessionTerminate` нет (молча игнорируется); перед ним сеансы
нужно завершить отдельно, например `ibcmd session terminate --remote=ssh://…` (см. раздел
о сеансах).

## Сеансы автономного сервера через SSH-шлюз

Замер 06.10.2026. Задача: [#188](https://github.com/IngvarConsulting/v8-runner-rust/issues/188). Платформа 8.3.27.2074, macOS.

**В shell шлюза команд сеансов нет.** Режимы shell — `help`, `common`, `options`,
`config`, `infobase-tools`; у `common` только `connect-ib` и `disconnect-ib`; режимов
`session` и `infobase` нет. В текстовом режиме `help session` и `session list` печатают
общую справку, `common session list` — `Ошибка разбора параметров командной строки`.

**Встроенный клиент раннера получает отказ.** Мерили кодом раннера: копия `src/` вне
репозитория с добавленным `#[ignore]`-тестом, который открывает `AgentSession` (russh,
канал `shell` без pty, `options set --show-prompt=no --output-format=json`,
`common connect-ib`) и шлёт команды по одной. Сессия открывается за ~10 мс; ответы:

```json
> session list
[{"error-type":"CommandFormatError","message":"Ошибка разбора параметра: ","type":"error"}]
> infobase session list
[{"error-type":"CommandFormatError","message":"Ошибка разбора параметра: ","type":"error"}]
> common session list
[{"error-type":"CommandFormatError","message":"Ошибка разбора параметров командной строки","type":"error"}]
> help session
[{"error-type":"CommandFormatError","message":"Ошибка разбора параметра: session","type":"error"}]
```

В JSON-режиме `help <режим>` отвечает `CommandFormatError` для любого режима, включая
существующие (`config`, `common`); справку даёт только текстовый режим. Канал `exec`
(`ssh … "session list"`) печатает ту же общую справку и закрывается кодом 255. Запрос pty
шлюз отклоняет (`PTY allocation request failed on channel 0`).

**Сеансами шлюз управляет через отдельную подсистему SSH `scop`.** Так работает
`ibcmd session list|terminate --remote=ssh://host:port`. Подменный SSH-сервер на russh
показал запрос `ibcmd`: аутентификация `none` с именем пользователя ОС, канал `session`,
`subsystem scop`; проксирование на настоящий шлюз дало обмен:

```
C->S VER 00000001\nFIN 00000000\n
S->C VER 00000001\nFIN 00000000\n
C->S MSG 000000010000\nCMD 000000010011list-session-info\nCAP 00000001000bformat=text\nBRK 00000001\nFIN 00000001\n
S->C MSG 00000001000800000001\nCMD 000000010011list-session-info\nBRK 00000001\nARG 000000010010result=integer:0\nBRK 00000001\nOUT 0000000107acsession : faa9bf94-… \n…\nFIN 00000001\n
```

Кадр — тег, 8 hex-цифр канала, 4 hex-цифры длины, тело. `list-session-info` отвечает
текстом `ключ : значение` (52 поля на сеанс: `session`, `session-id`, `infobase`,
`app-id`, `started-at`, …); `CAP format=json` игнорируется — снова текст.
`terminate-session` несёт аргумент `data` с двоичным сериализованным телом (`VAL`,
0xb8–0xd8 байт запроса, двоичный ответ) — текстового протокола у завершения нет. Тот же
обмен воспроизвёл свой клиент на russh (`request_subsystem("scop")`, без pty): список
сеансов пришёл за ~1,5 мс. Аутентификацию `none` подсистема принимает с любым именем
(`""`, `x`, `admin`); пароль `""` — только для пустого имени (так же у shell раннера:
`authenticate_password("admin", "")` → `AuthenticationRejected`, `""` — принят). Это
отличается от замера 13.09 («шлюз — любое имя с пустым паролем»).

| Командная строка | Код | Время | Вывод |
| --- | --- | --- | --- |
| `ibcmd session list --pid=<pid>` | 0 | 1,0 с | текст `ключ : значение`, один сеанс `app-id: 1CV8C` |
| `ibcmd session list --remote=ssh://127.0.0.1:18343` без tty, ключ хоста неизвестен | 139 (SIGSEGV) | 1,0 с | `Не удалось установить подлинность узла '127.0.0.1'. … [y/n] :` |
| то же с tty (`expect`, ответ `y`) | 0 | ~1 с | ключ дописан в `~/.ssh/known_hosts`, список сеансов |
| `ibcmd session list --remote=ssh://…` без tty, ключ уже известен | 0 | 1,0 с | список сеансов |
| `ibcmd session terminate --session=<uuid> --error-message="probe 188" --remote=ssh://…` | 0 | 1,0 с | пусто; сеанс исчез из списка, процесс тонкого клиента остался жив |
| `ibcmd session terminate --session=00000000-0000-0000-0000-000000000001 --remote=ssh://…` (нет такого сеанса) | 0 | 1,0 с | пусто — отказа нет |
| то же через `--pid` | 0 | 1,0 с | пусто |
| `ibcmd session info --session=<несуществующий uuid> --remote=ssh://…` | 0 | 1,0 с | пусто |
| `ibcmd session terminate --session=not-a-uuid --remote=ssh://…` | 2 | 1,0 с | `Не указано значение параметра: session` |
| `ibcmd session list --remote=ssh://… --user= --password=` | 2 | 1,0 с | `Ошибка разбора параметра: --user=` |

```sh
B=<рабочий каталог>; T=$B/runner/target/debug/deps/v8_runner-<hash>
V8_GATE_PROBE=127.0.0.1:18343 V8_GATE_TRANSCRIPT=$B/g2.transcript \
  V8_GATE_CMDS='help config;;help session;;session list;;infobase session list;;common session list' \
  $T --ignored --exact platform::agent::tests::gate_command_probe --nocapture
expect gate.exp "help" "help common" "help infobase-tools" "session list"   # ssh -T -p 18343 -l "" 127.0.0.1
expect sshexec.exp "session list"                                          # канал exec
$P/ibcmd session list --remote=ssh://127.0.0.1:18343 </dev/null
$P/ibcmd session terminate --session=<uuid> --error-message="probe 188" --remote=ssh://127.0.0.1:18343 </dev/null
sniff $B/hostkey   # подменный сервер на 127.0.0.1:18344, проксирует subsystem в 18343
$P/ibcmd session list --remote=ssh://127.0.0.1:18344 </dev/null
scop 18343 "" none list-session-info format=json   # свой клиент russh, subsystem scop
```

**Вывод для потребителей.** Для #212: у текущего исполнителя раннера (агентский shell
через встроенный клиент) `sessions list|terminate` автономного сервера — отказ с
причиной: shell шлюза не знает команд сеансов (`CommandFormatError`). Канал к сеансам у
шлюза есть, но это другая подсистема SSH (`scop`) с собственным кадрированием; список
сеансов по ней встроенный клиент получает (замерено russh, текст `ключ : значение`), а
завершение требует двоичного тела, формат которого не описан — делать его в раннере
значит разбирать закрытый протокол. Работающий исполнитель без этого — процесс
`ibcmd session list|terminate --remote=ssh://<gate>` (или `--pid` на той же машине):
работает без терминала, если ключ хоста уже в `~/.ssh/known_hosts`; иначе без tty
`ibcmd` падает с SIGSEGV (код 139). Ответ — текст `ключ : значение` без JSON;
`terminate` несуществующего сеанса возвращает 0 с пустым выводом, поэтому успех
завершения доказывает только повторный `list`.

### Что не удалось или не мерили (#178, #188)

- Порт по умолчанию 1541 подтверждён только адресом в ошибке клиента и `ibsrv --help`:
  сервер на 1541 в замере не поднимался.
- Тонкий клиент открывает GUI; кроме факта сеанса и кода при неверном порте ничего не
  снималось, окна закрывались по таймауту.
- База без пользователей; поведение `scop` и `ibcmd --remote` с пользователями ИБ не мерили.

## Переименование расширения

Замер 06.10.2026. Задача: [#185](https://github.com/IngvarConsulting/v8-runner-rust/issues/185). Платформа 8.3.27.2074, macOS.

Стенд — файловая база с основной конфигурацией из `tests/fixtures/designer/configuration`
и расширением `Расширение1` из `tests/fixtures/designer/extension`. В копию расширения
добавлена собственная константа `Расш1_Константа1` (строка), в копию основной конфигурации —
модуль управляемого приложения и серверный модуль: по `/C` они пишут или читают константу и
выводят в файл список `РасширенияКонфигурации.Получить()` (имя и уникальный идентификатор).
Константа записана через `1cv8 ENTERPRISE /C` (толстый клиент), значение
`ДанныеРасширения1`. Перед каждым сценарием база восстанавливалась из одного снимка.
Переименованная копия расширения отличается от установленной только `<Name>Расширение2</Name>`
в `Configuration.xml` и именем в `ConfigDumpInfo.xml`.

Данные проверялись тем же клиентом после каждого шага (`/C "read|..."`, ≈3,7 с).

| Вызов | Код | Что делает платформа | Данные `Расширение1` |
| --- | --- | --- | --- |
| Конфигуратор `/LoadConfigFromFiles <копия> -Extension Расширение1`, затем `/UpdateDBCfg -Extension Расширение1` | 0, 0 | переименовывает на месте: одно расширение `Расширение2`, тот же идентификатор `7e628dbe-…` | сохранились |
| Конфигуратор `/LoadConfigFromFiles <копия> -Extension Расширение2`, затем `/UpdateDBCfg -Extension Расширение2` | 0, **1** | загрузка заводит второе расширение `Расширение2` (новый идентификатор, `hash-sum: "AAAA…"`); обновление отказывает: «Обнаружено пересечение внутренних идентификаторов с расширением конфигурации 'Расширение1'» | сохранились; пустое `Расширение2` остаётся в базе |
| `ibcmd infobase config import --extension=Расширение1 <копия>`, затем `config apply --extension=Расширение1 --force` | 0, 0 | как у Конфигуратора: переименование на месте | сохранились |
| `ibcmd infobase config import --extension=Расширение2 <копия>`, затем `config apply --extension=Расширение2 --force` | 0, **255** | второе расширение; `apply` отказывает тем же текстом про пересечение идентификаторов | сохранились; пустое `Расширение2` остаётся |
| Агент: `config load-config-from-files --dir=ext2 --extension=Расширение1`, `config update-db-cfg --extension=Расширение1` | success, success | переименование на месте | сохранились |
| Агент: то же с `--extension=Расширение2` | success, **error** | второе расширение; `update-db-cfg` отвечает `"error-type": "UnknownError"` с тем же текстом про пересечение | сохранились |

Ни один исполнитель не предупреждает, что `Name` в файлах отличается от имени в
`-Extension`/`--extension`: загрузка отвечает пустым `/Out` и `rc=0`.

**Промежуточное состояние при переименовании на месте.** После загрузки и до
`/UpdateDBCfg` расширение адресуется только по старому имени: `ibcmd ... extension list`
показывает `name: "Расширение1"` с прежним `hash-sum`, `/DumpConfigToFiles -Extension
Расширение2` отвечает `rc=1`, «…расширение конфигурации с указанным именем не найдено:
Расширение2», а выгрузка по старому имени отдаёт `<Name>Расширение2</Name>`. После
`/UpdateDBCfg` — наоборот: старое имя даёт `rc=1` с тем же текстом, новое работает.
Агент в ответе на несуществующее имя пишет пустое имя: «…расширение конфигурации '' не
найдено», `"error-type": "ExtensionNotFound"`.

**Рецепт владельца: удалить, затем загрузить под новым именем.** Исполнители ведут себя
по-разному.

| Исполнитель | Удаление активного расширения с данными | Удаление после снятия активности | Итог рецепта |
| --- | --- | --- | --- |
| Конфигуратор `/DeleteCfg -Extension Расширение1` | `rc=0` без вопроса; `/Out`: «Объект удален: Константа.Расш1_Константа1», «Расширение конфигурации было успешно удалено из информационной базы: Расширение1» | не нужно | загрузка и обновление под `Расширение2` — `rc=0`, `rc=0`; константа есть, **значение пустое** |
| `ibcmd infobase config extension delete --name=Расширение1` | `rc=1`: «[ERROR] Расширение конфигурации 'Расширение1' активно и содержит данные. Удаление расширений с данными допускается только после снятия признака активности.», и следом строка «[INFO] Удаление расширения: 'Расширение1' успешно завершено»; расширение остаётся | после `extension update --active=no` спрашивает «Принять изменения и продолжить обновление [y/n]». Без терминала: `rc=255`, «Операция не выполнена!» (stdin закрыт) или «Invalid seek» (ответ через канал). С терминалом (`expect`) и ответом `y` — `rc=0`. Ключа подтверждения у `extension delete` нет | после удаления через терминал `import` + `apply` под `Расширение2` — `rc=0`; **значение пустое** |
| Агент `config extensions delete --extension=Расширение1` | `"error-type": "ExtensionWithDataIsActive"`, тот же текст, что у `ibcmd` | после `config extensions properties set --active=no`: `"error-type": "UnknownError"`, «Запрещено использование окон. Передан ключ DisableStartupDialogs.»; расширение остаётся, неактивным | не выполнен |

Сразу после `/DeleteCfg` константы в метаданных сеанса нет; после новой загрузки она
создаётся заново («Новый объект: Константа.Расш1_Константа1») с новым идентификатором
расширения и без прежнего значения. Неактивное расширение `РасширенияКонфигурации.Получить()`
по-прежнему перечисляет, но его константы в метаданных сеанса нет.

Попутные наблюдения:

- `/LoadConfigFromFiles <dir> -Extension X /UpdateDBCfg -Extension X` одной командной
  строкой — `rc=1`, `/Out`: «Ошибка в параметрах командной строки.». Те же команды двумя
  запусками проходят.
- Расширение, загруженное Конфигуратором, ставится с `safe-mode: yes` и
  `unsafe-action-protection: yes`.
- Дважды `ibcmd` без `--data` отказал: `rc=254`, «Ошибка блокировки каталога данных
  сервера. Рабочий каталог заблокирован процессом: <pid>». К моменту проверки процесса с
  этим pid уже не было. С собственным `--data=<каталог>` отказ не повторился.

```text
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /LoadConfigFromFiles <ext> -Extension Расширение1 /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /UpdateDBCfg -Extension Расширение1 /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /LoadConfigFromFiles <ext2> -Extension Расширение1 /Out <f>   # rc=0, переименование
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /LoadConfigFromFiles <ext2> -Extension Расширение2 /Out <f>   # rc=0, второе расширение
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /UpdateDBCfg -Extension Расширение2 /Out <f>                  # rc=1, пересечение идентификаторов
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /DeleteCfg -Extension Расширение1 /Out <f>                    # rc=0, данные удалены
ibcmd infobase config extension list   --data=<d> --db-path=<ib>
ibcmd infobase config import           --data=<d> --db-path=<ib> --extension=Расширение2 <ext2>
ibcmd infobase config apply            --data=<d> --db-path=<ib> --extension=Расширение2 --force           # rc=255 при живом Расширение1
ibcmd infobase config extension delete --data=<d> --db-path=<ib> --name=Расширение1                        # rc=1, активно и содержит данные
ibcmd infobase config extension update --data=<d> --db-path=<ib> --name=Расширение1 --active=no
1cv8 DESIGNER /F <ib> /AgentMode /AgentListenAddress 127.0.0.1 /AgentPort <p> /AgentBaseDir <dir> /AgentSSHHostKeyAuto
ssh -T -l '' -p <p> 127.0.0.1    # SSH_ASKPASS с пустым паролем, SSH_ASKPASS_REQUIRE=force
  config load-config-from-files --dir=ext2 --extension=Расширение2
  config update-db-cfg --extension=Расширение2
  config extensions delete --extension=Расширение1
1cv8 ENTERPRISE /F <ib> /DisableStartupDialogs /DisableStartupMessages /C "write|<значение>|<файл>|Расш1_Константа1"
```

Каждый вызов Конфигуратора занял 3–3,5 с, `ibcmd` — 5–6,5 с.

**Вывод для потребителей (#218).** Платформа сама не отказывает, когда `Name` в файлах
отличается от имени установленного расширения. Исход задаёт имя, переданное в
`-Extension`. Со старым именем расширение переименовывается на месте с сохранением данных.
С новым именем появляется второе расширение, а обновление базы отказывает с `rc=1`
(`ibcmd` — `rc=255`). Пустое второе расширение после этого остаётся в базе, его нужно
удалять отдельно. Значит, раннеру нужна своя проверка до запуска: сравнить `Name` из
`Configuration.xml` с именем установленного расширения. Рецепт «удалить, затем загрузить»
работает, но **данные расширения при удалении теряются безвозвратно**: Конфигуратор
удаляет их без вопроса и пишет в `/Out` «Объект удален: …». Предупреждение должно говорить
именно об этом. Другие исполнители не дают пройти рецепт без вмешательства: `ibcmd` и
агент отказываются удалять активное расширение с данными. После снятия активности `ibcmd`
просит подтверждение в терминале, а агент упирается в запрет окон.

## Частичная загрузка в базу под хранилищем

Замер 06.10.2026. Задача: [#186](https://github.com/IngvarConsulting/v8-runner-rust/issues/186). Платформа 8.3.27.2074, macOS.

Стенд — файловая база с основной конфигурацией из фикстуры и локальное файловое
хранилище, созданное из этой базы (`/ConfigurationRepositoryCreate` подключает базу
сразу). Пользователь хранилища `Админ` захватил `Справочник.Справочник1`. В копии
XML-файлов изменён `Comment` у `Справочник1` (захвачен) и у `Перечисление1` (не захвачен).
Перед каждым сценарием база и хранилище восстанавливались из одного снимка.

| Загрузка | Код | `/Out` |
| --- | --- | --- |
| `-partial -listFile`, незахваченный объект | 1 | «Загрузка невозможна: объект метаданных Enum.Перечисление1 не захвачен в хранилище!» |
| `-partial -listFile`, захваченный объект | 0 | пусто; изменение попадает в конфигурацию (проверено выгрузкой) |
| `-files "Enums/Перечисление1.xml"` без `-partial` | 1 | тот же текст |
| `-files "Catalogs/Справочник1.xml"` без `-partial` | 0 | пусто |
| `-files "<захваченный>,<незахваченный>" -partial` | 1 | тот же текст; **не загружается ничего**, захваченный объект тоже (проверено выгрузкой) |
| список из захваченного и двух незахваченных | 1 | называется **один** объект: `Constant.Константа1`, не первый по списку |
| незахваченный объект, файл которого не изменён | 1 | тот же отказ: проверяется захват, а не наличие изменений |
| полная `/LoadConfigFromFiles <dir>` (изменённые или исходные файлы) | 1 | «Загрузка невозможна: текущая конфигурация помещена в хранилище.» |
| `ibcmd infobase config import <dir>` | 255 | «[ERROR] Операция невозможна: информационная база подключена к хранилищу конфигурации» |
| `ibcmd infobase config import files --partial --base-dir=<dir> Enums/Перечисление1.xml` | 255 | «[ERROR] Импорт файлов конфигурации из XML завершен с ошибкой. Загрузка невозможна: объект метаданных Enum.Перечисление1 не захвачен в хранилище!» |
| то же для `Catalogs/Справочник1.xml` | 0 | «[INFO] Импорт файлов конфигурации из XML успешно завершен» |
| вторая база, подключена пользователем `Другой`, `-partial` по захваченному `Админ` объекту | 1 | «Загрузка невозможна: объект метаданных Catalog.Справочник1 не захвачен в хранилище!» |

**Проверка захвата локальная.** Без `/ConfigurationRepositoryF/N/P` каждый вызов
Конфигуратора начинает `/Out` строкой «Соединение с хранилищем конфигурации не
установлено», но отвечает так же, с тем же кодом. С параметрами хранилища этой строки нет,
остальное совпадает. Решение о захвате принимается по сведениям, которые хранит сама база.
Захват другим пользователем для этой базы выглядит как «не захвачен», без имени владельца.

**Как узнать захват до запуска — команды только для чтения не нашлось.**

| Кандидат | Результат |
| --- | --- |
| `/ConfigurationRepositoryReport <f> -ReportFormat txt` и с `-GroupByObject` | `rc=0`, только история версий и комментарии; о захватах ни слова |
| `/DumpConfigToFiles`, `ConfigDumpInfo.xml` | база под хранилищем и без него отличаются только `configVersion`; сведений о захвате нет |
| `ibcmd infobase config export info` / `export status --base=<ConfigDumpInfo.xml>` | `rc=0`, `ConfigDumpInfo` и список изменений; захвата нет |
| агент (`help config`) | команд хранилища в агенте нет (по справочнику `references/1c/designer-agent/`) |
| `/ConfigurationRepositoryLock -objects <xml>` | о чужом захвате сообщает: `rc=1`, «Объект захвачен для редактирования другим пользователем: Справочник.Справочник1 (Админ)», «Ошибка захвата объектов в хранилище». Но это не проверка: свободный объект команда захватывает («Объект захвачен для редактирования: Справочник.Справочник1», `rc=0`) |

Ближе всего к проверке сама частичная загрузка: она атомарна. При любом незахваченном
объекте не загружается ничего, ответ — `rc=1` с текстом «не захвачен в хранилище». Однако
она называет только один объект, а различить отказы можно лишь по прозе.

```text
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /ConfigurationRepositoryF <repo> /ConfigurationRepositoryN Админ /ConfigurationRepositoryP <pwd> /ConfigurationRepositoryCreate /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /ConfigurationRepositoryF <repo> /ConfigurationRepositoryN Админ /ConfigurationRepositoryP <pwd> /ConfigurationRepositoryLock -objects <lock.xml> /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /LoadConfigFromFiles <dir> -partial -listFile <list.txt> /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /LoadConfigFromFiles <dir> -files "Catalogs/Справочник1.xml,Enums/Перечисление1.xml" -partial /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /LoadConfigFromFiles <dir> /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /ConfigurationRepositoryF <repo> /ConfigurationRepositoryN Админ /ConfigurationRepositoryP <pwd> /ConfigurationRepositoryReport <f.txt> -GroupByObject -ReportFormat txt /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /ConfigurationRepositoryF <repo> /ConfigurationRepositoryN Админ /ConfigurationRepositoryP <pwd> /ConfigurationRepositoryAddUser -User Другой -Pwd <pwd2> -Rights LockObjects /Out <f>
1cv8 DESIGNER /F <ib2> /DisableStartupDialogs /ConfigurationRepositoryF <repo> /ConfigurationRepositoryN Другой /ConfigurationRepositoryP <pwd2> /ConfigurationRepositoryBindCfg -forceReplaceCfg /Out <f>
ibcmd infobase config import --data=<d> --db-path=<ib> <dir>
ibcmd infobase config import files --data=<d> --db-path=<ib> --base-dir=<dir> --partial Enums/Перечисление1.xml
ibcmd infobase config export info   --data=<d> --db-path=<ib>
ibcmd infobase config export status --data=<d> --db-path=<ib> --base=<ConfigDumpInfo.xml>
```

`lock.xml`, который принял `/ConfigurationRepositoryLock`:

```xml
<Objects xmlns="http://v8.1c.ru/8.3/config/objects" version="1.0">
	<Object fullName="Справочник.Справочник1" includeChildObjects="true"/>
</Objects>
```

Каждый вызов Конфигуратора занял 3–3,7 с, `ibcmd` — 5–6 с.

**Вывод для потребителей (#220).** В базе под хранилищем полная загрузка запрещена всегда:
`rc=1` у Конфигуратора, `rc=255` у `ibcmd`. Частичная (`-partial`/`-files`, `ibcmd import
files --partial`) проходит только для объектов, которые захватила именно эта база. Отказ
атомарный, называет один объект и не требует связи с хранилищем. Пакетной команды,
которая только читает состояние захватов, не нашлось: отчёт хранилища, выгрузка и
`export info/status` о захватах молчат. `/ConfigurationRepositoryLock` о чужом захвате
сообщает, но свободный объект при этом захватывает, что противоречит решению «раннер не
захватывает». Выполнимая проверка до запуска сводится к одному: база подключена к
хранилищу, значит полная загрузка заведомо отказывает. Признак подключения — отказ полной
загрузки или `ibcmd import` с «подключена к хранилищу конфигурации». Отдельный вызов,
который только читает этот признак, тоже не мерен. Захват отдельных объектов раннер до
запуска не знает. Его отказ приходит от самой частичной загрузки: она безопасна, потому
что атомарна, но распознаётся только по прозе.

## Кластер в Docker: стенд

Разделы о `rac`, `CREATEINFOBASE` и поколении у базы СУБД сняты 06.10.2026 на другой сборке —
**8.5.4.1878**: кластер (`ragent`, `ras`) и PostgreSQL 17.10 из сборки Postgres Pro для 1С в Docker (`linux/amd64` под эмуляцией
на macOS), `rac` и `ibcmd` внутри контейнеров, `1cv8` на Mac той же сборки. Сервера 8.3.27 под
рукой не было; на 8.3.27 эти факты не перепроверены. В клиентской поставке 8.5.4.1878 для macOS
утилиты `rac` нет. Базы без пользователей ИБ. Пароли в командных строках заменены на `<пароль>`.
Выводы этих разделов о составе ключей и учётных данных относятся к 8.5.4 и не переносятся на 8.3.27
без перепроверки.

## rac: команды, ключи и учётные данные

Замер 06.10.2026. Задача: [#180](https://github.com/IngvarConsulting/v8-runner-rust/issues/180). Платформа 8.5.4.1878 (кластер в Docker linux/amd64 под эмуляцией на macOS, PostgreSQL 17.10).

**Справка.** `rac help` (rc=0) перечисляет режимы `agent, cluster, manager, server, process, service, infobase,
connection, session, lock, rule, profile, counter, limit, service-setting-server, binary-data-storage,
service-setting-cluster`. Общий аргумент — `<host>[:<port>]`, по умолчанию `localhost:1545`.

- `session`: общие `--cluster=<uuid>` (обязательный), `--cluster-user`, `--cluster-pwd`, `--cluster-key=<path>`;
  команды `info --session=<uuid> [--licenses]`, `list [--infobase=<uuid>] [--licenses]`,
  `terminate --session=<uuid> [--error-message=<string>]`, `interrupt-current-server-call --session=<uuid> [--error-message]`.
- `infobase`: те же общие ключи кластера; команды `info`, `summary info|list|update`, `create`, `update`, `drop`.
  **Ключей `--infobase-user`/`--infobase-pwd` в 8.5.4 нет ни у одной команды.** `info`, `update`, `drop`
  принимают `--infobase=<uuid>` **или** `--name=<name>`.
- `infobase create`: обязательные по справке `--name`, `--dbms=MSSQLServer|PostgreSQL|IBMDB2|OracleDatabase`,
  `--db-server`, `--db-name`, `--locale`; необязательные `--create-database`, `--db-schema`, `--db-user`, `--db-pwd`,
  `--descr`, `--date-offset`, `--security-level`, `--denied-from/-to/-message/-parameter`, `--permission-code`,
  `--sessions-deny=on|off`, `--scheduled-jobs-deny=on|off`, `--license-distribution=deny|allow` и прочие.
- `infobase update`: те же поля без обязательных плюс `--data-version=<version>` (оптимистичная блокировка).
- `infobase drop`: `--infobase|--name`, `--drop-database`, `--clear-database`.
- `agent`: `--agent-user`, `--agent-pwd`, `--agent-key`; `admin list|register|remove`. У `register` ключи
  `--name`, `--pwd`, `--publickey`, `--descr`, `--auth=pwd[,os,publickey]`, `--os-user`, `--read-only=yes|no`,
  `--password-auth-from-localhost-only=yes|no`, `--data-version`. У `cluster admin` — тот же набор.
- `cluster list` ключа `--agent-user` не принимает: `Ошибка разбора параметра: --agent-user=…`, rc=255.

**Ответы `infobase create`** (списки администраторов пусты):

| Вызов | rc | Ответ |
| --- | --- | --- |
| полный, `--create-database`, БД нет | 0 | `infobase : <uuid>` и `data-version : <64 hex>`; БД создана, 82 таблицы; ~7 с |
| то же имя повторно | 255 | `Информационная база v8rm_r1 уже зарегистрирована в кластере серверов 1С:Предприятия` |
| без `--locale` / `--dbms` / `--cluster` | 255 | `Ошибка разбора параметра: locale` (`dbms`, `cluster`) |
| без `--db-user`/`--db-pwd` | 255 | `Ошибка операции администрирования` / `Сервер баз данных не обнаружен` / `…fe_sendauth: no password supplied` |
| без `--create-database`, БД нет | 255 | `…Сервер баз данных не обнаружен` / `…база данных "v8rm_r3" не существует` |
| `--create-database`, пустая БД уже есть | 0 | создана, 82 таблицы в существующей БД |
| без `--create-database`, пустая БД есть | 0 | создана, 82 таблицы — структура пишется и без ключа |
| `--create-database`, БД уже держит другую ИБ, новое имя | 0 | **молча** зарегистрирована вторая ИБ на той же БД; конфигурация в БД сохранилась (поколение `ibcmd` до и после одно и то же) |

У созданной через `rac` базы по умолчанию `license-distribution: deny`, `scheduled-jobs-deny: off`.

**Запрет сеансов.** `infobase update --sessions-deny=on --denied-message=… --permission-code=… --scheduled-jobs-deny=on`
— rc=0, пустой вывод; `infobase info` показывает `sessions-deny : on`, `denied-message : "Обновление v8rm"`,
`permission-code : "v8rmcode"`, `scheduled-jobs-deny : on`. Снятие — `--sessions-deny=off --denied-message= --permission-code= --scheduled-jobs-deny=off`,
пустые значения поля очищают. Ошибки: без `--infobase`/`--name` — `Ошибка разбора параметра: infobase`;
неизвестное имя — `Информационная база с указанным идентификатором не найдена`; устаревший `--data-version` —
`Данные были изменены или удалены другим пользователем. Необходимо перечитать данные и начать редактировать заново.`;
все rc=255.

**Сеансы.** `session list --infobase=<uuid>` после `rac`-операций показывает сеанс самого RAS (`app-id : RAS`,
`user-name :` пусто). `session terminate --session=<uuid> --error-message=…` — rc=0, сеанс исчез. Несуществующий
UUID — `Сеанс с указанным идентификатором не найден`, rc=255; без `--session` — `Ошибка разбора параметра: session`.

**Администраторы.** Все отказы — rc=255.

| Состояние | Операция | Без учётных данных | С учётными данными |
| --- | --- | --- | --- |
| оба списка пусты | любая (`infobase create/info/update/drop`, `session list/terminate`, `cluster admin list`, `agent admin list`) | rc=0 | — |
| есть администратор кластера | `infobase summary list/info/update/create`, `session list/terminate`, `cluster admin list/register` | `Администратор кластера не аутентифицирован` | `--cluster-user/--cluster-pwd` → rc=0; неверный пароль — тот же текст |
| есть администратор кластера | `cluster list`, `cluster info` | rc=0 | — |
| есть администратор центрального сервера | `agent admin list/register` | `Администратор центрального сервера не аутентифицирован` | `--agent-user/--agent-pwd` → см. ниже |
| есть администратор центрального сервера | `cluster list`, `infobase …` с ключами кластера | без изменений (агентский пароль не нужен) | — |

`register` по умолчанию ставит `password-auth-from-localhost-only : yes`, и это ограничение действует:

| Где `rac` → куда | админ кластера (`yes`) | админ центрального сервера (`yes`) | оба с `=no` |
| --- | --- | --- | --- |
| `ras` → `localhost:1545` | rc=0 | rc=255 `Ошибка операции администрирования` / `Аутентификация по паролю доступна только с локальной машины` | rc=0 |
| `ras` → `ras:1545` | rc=0 | rc=255, тот же текст | rc=0 |
| `srv` → `ras:1545` | rc=255 `Аутентификация по паролю доступна только с локальной машины` | rc=255, тот же текст | rc=0 |
| `srv` → временный `ras` в `srv` на `localhost:1546` | rc=0 | rc=0 | rc=0 |

То есть «локально» для администратора кластера — `rac` на одной машине с RAS, для администратора центрального
сервера — RAS на одной машине с `ragent`. Администратор с `=yes`, зарегистрированный через удалённый RAS, этим же
RAS уже не удаляется — пришлось поднимать временный `ras` внутри `srv` (запасной выход, процесс потом остановлен).

**IPv6.** `ras` слушает только IPv4 (`0.0.0.0:1545`, в `/proc/net/tcp6` слушателей нет), поэтому проверялся разбор
адреса: внутри `ras` поднят `nc -6 -l ::1 1546` (контроль `nc -6 ::1 1546` доходит, 3 байта).

| Адрес | Ответ `rac cluster list` | Байт у слушателя `::1` |
| --- | --- | --- |
| `[::1]:1546`, `[::1]:1545`, `[::1]`, `[0:0:0:0:0:0:0:1]:1545` | `Ошибка соединения с сервером` / `[:0` (`[0:0`) / `Already open`, rc=255 | 0 |
| `::1:1546`, `::1` | `Ошибка соединения с сервером` / `Connection refused`, rc=255 | 0 |
| `ip6-localhost:1546` (в `/etc/hosts` = `::1`) | `…ip6-localhost:1546` / `Host not found (authoritative)`, rc=255 | 0 |
| контроль: `127.0.0.1:1547` с `nc -4 -l` | соединение есть (32 байта получено) | — |

Скобки `rac` 8.5.4.1878 не разбирает: адрес режется по первому `:`, хостом становится `[`, портом 0. С Mac
`rac` проверить нельзя — его нет в клиентской поставке на macOS; кроме того, Docker публикует 1545 только на
`127.0.0.1` (`nc -6 -z ::1 1545` с Mac — rc=1).

```sh
# rac — в контейнере: docker compose exec -T ras rac …
rac help ; rac help session ; rac help infobase ; rac help agent ; rac help cluster
rac cluster list localhost:1545
rac infobase create --cluster=<cl> --create-database --name=v8rm_r1 --dbms=PostgreSQL --db-server=db \
  --db-name=v8rm_r1 --db-user=postgres --db-pwd=<пароль> --locale=ru localhost:1545
rac infobase update --cluster=<cl> --infobase=<ib> --sessions-deny=on --denied-message="Обновление v8rm" \
  --permission-code=v8rmcode --scheduled-jobs-deny=on localhost:1545
rac infobase update --cluster=<cl> --name=v8rm_r1 --sessions-deny=off --denied-message= --permission-code= \
  --scheduled-jobs-deny=off localhost:1545
rac infobase info --cluster=<cl> --name=v8rm_r1 localhost:1545
rac session list --cluster=<cl> --infobase=<ib> localhost:1545
rac session terminate --cluster=<cl> --session=<uuid> --error-message="v8rm terminate" localhost:1545
rac cluster admin register --cluster=<cl> --name=v8rm_cadm --pwd=<пароль> --auth=pwd localhost:1545
rac infobase summary list --cluster=<cl> --cluster-user=v8rm_cadm --cluster-pwd=<пароль> localhost:1545
rac agent admin register --name=v8rm_aadm --pwd=<пароль> --auth=pwd localhost:1545
rac agent admin list --agent-user=v8rm_aadm --agent-pwd=<пароль> localhost:1545
# временный RAS внутри srv для локальной аутентификации:
docker compose exec -d -u onec srv /opt/1cv8/x86_64/8.5.4.1878/ras cluster --port=1546 localhost:1540
rac infobase drop --cluster=<cl> --name=v8rm_r1 --drop-database localhost:1545
rac cluster list '[::1]:1545'
```

**Вывод для потребителей.** Учётные данные (#212): `cluster list`/`cluster info` не требуют ничего; всё про базы и
сеансы (`infobase create|info|update|drop`, `session list|terminate`) требует администратора кластера, если его
список не пуст, и только его — пароль центрального сервера этим операциям не нужен и `rac` их даже не примет
(`cluster list --agent-user` — ошибка разбора). Администратор центрального сервера нужен только для `agent …`.
Учётных данных пользователя ИБ `rac` 8.5.4 не принимает вовсе (на 8.3.27 не проверено). Отказ аутентификации различим только по тексту
(`…не аутентифицирован`, `Аутентификация по паролю доступна только с локальной машины`), код всегда 255; учитывать
`password-auth-from-localhost-only=yes` по умолчанию: рабочая учётная запись для удалённого RAS должна быть
зарегистрирована с `=no`. Запасной путь создания (#204): `rac infobase create … --create-database --locale=…`
печатает `infobase : <uuid>` — это и есть признак «создана»; «уже была» — rc=255 с текстом «уже зарегистрирована»;
`--create-database` не защищает от регистрации второй ИБ на чужой непустой БД. Удаление с `--drop-database` сносит
БД, даже если на неё зарегистрирована ещё одна ИБ. Адрес RAS для `rac` — только IPv4 или имя, разрешаемое в IPv4.

## CREATEINFOBASE для кластера

Замер 06.10.2026. Задача: [#181](https://github.com/IngvarConsulting/v8-runner-rust/issues/181). Платформа 8.5.4.1878 (кластер в Docker linux/amd64 под эмуляцией на macOS, PostgreSQL 17.10).

Запуск с Mac: `1cv8 CREATEINFOBASE "<строка>" /Out <файл> /DisableStartupDialogs`. Каждый вызов — 11–13 с
(первый — 53 с). `/Out` — UTF-8 с BOM (`ef bb bf`). Лицензия для создания не потребовалась.

| Строка (относительно полной `Srvr;Ref;DBMS;DBSrvr;DB;DBUID;DBPwd;CrSQLDB=Y;Locale=ru`) | rc | `/Out` | Последствия |
| --- | --- | --- | --- |
| полная | 0 | `Создание информационной базы ("<вся строка соединения>") успешно завершено` | ИБ зарегистрирована, БД создана |
| без `Locale` (строка из задачи) | 1 | `Ошибка установки или изменения национальных настроек информационной базы` / `Порядок сортировки не поддерживается базой данных` | **БД создана и брошена** (16 таблиц), ИБ не зарегистрирована |
| без `Locale`, но `LANG=LC_ALL=ru_RU.UTF-8` | 1 | то же | то же |
| `Locale=en` | 1 | то же | то же |
| повтор после сбоя на ту же полусозданную БД | 1 | `Поколение информационной базы не найдено: db:head` | **ИБ зарегистрирована** на битую БД |
| без `DB` | 1 | `Неверные или отсутствующие параметры соединения, необходимые для создания информационной базы` | ничего |
| без `DBSrvr` | 1 | то же | ничего |
| без `DBMS` | 1 | `Неверные или отсутствующие параметры соединения с информационной базой` / `Неверный тип сервера баз данных: ''` | ничего |
| без `Ref` | 1 | `Неверные или отсутствующие параметры соединения с информационной базой` | ничего |
| без `DBPwd` | 1 | `Сервер баз данных не обнаружен` / `…fe_sendauth: no password supplied` | ничего |
| без `DBUID` | 1 | `Сервер баз данных не обнаружен` / `…пользователь "onec" не прошёл проверку подлинности (по паролю)` — подставлен пользователь ОС процесса сервера | ничего |
| без `CrSQLDB`, БД нет | 1 | `Сервер баз данных не обнаружен` / `…база данных "v8rm_m7" не существует` | ничего |
| `CrSQLDB=N`, БД нет | 1 | то же (`"v8rm_m8"`) | ничего |
| `CrSQLDB=Y`, пустая БД есть | 0 | «успешно завершено» | структура создана в существующей БД |
| `CrSQLDB=N`, пустая БД есть | 0 | «успешно завершено» | то же |
| `CrSQLDB=Y`, БД уже держит ИБ, новый `Ref` | 0 | «успешно завершено» | вторая ИБ на той же БД, конфигурация не тронута (поколение то же) |
| `Ref` уже зарегистрирован (любые `DB`, `CrSQLDB=Y|N`) | 1 | `Указанная информационная база уже существует.` | ничего; проверка идёт **до** аутентификации администратора |

`SchJobDn`: с `SchJobDn=Y` у базы `scheduled-jobs-deny : on`, без ключа — `off` (по `rac infobase info`);
`SchJobDn=N` отдельно не мерялся. `license-distribution` у созданной так базы — `deny`.

**`SUsr`/`SPwd`.** При пустом списке администраторов кластера не нужны. При непустом:

| Строка | rc | `/Out` |
| --- | --- | --- |
| без `SUsr/SPwd` | 1 | `Запрещено использование окон. Передан ключ DisableStartupDialogs.` (клиент хотел спросить пароль) |
| `SUsr` верный, `SPwd` неверный | 1 | `Администратор кластера не аутентифицирован. Создание информационной базы невозможно` / `Администратор кластера не аутентифицирован` |
| верные | 0 | «успешно завершено» |
| `SUsr` = администратор центрального сервера (не кластера) | 1 | как при неверном пароле |

Флаг `password-auth-from-localhost-only=yes` у администратора кластера этот путь с Mac (через проброс портов
Docker) не остановил. Администратор центрального сервера для `CREATEINFOBASE` не нужен.

**Утечка пароля.** Строка успеха в `/Out` повторяет всю строку соединения: `DBPwd` и `SPwd` попадают в файл
открытым текстом.

```sh
/opt/1cv8/8.5.4.1878/1cv8 CREATEINFOBASE \
  "Srvr=onec.localhost:1541;Ref=v8rm_x2;DBMS=PostgreSQL;DBSrvr=db;DB=v8rm_x2;DBUID=postgres;DBPwd=<пароль>;CrSQLDB=Y;SchJobDn=Y;Locale=ru" \
  /Out <файл> /DisableStartupDialogs
# с администратором кластера:
"…;CrSQLDB=Y;Locale=ru;SUsr=v8rm_cadm;SPwd=<пароль>"
```

**Вывод для #204.** Строка для кластера:
`Srvr=<host:port>;Ref=<ИБ>;DBMS=PostgreSQL;DBSrvr=<сервер СУБД>;DB=<БД>;DBUID=<польз.>;DBPwd=<пароль>;CrSQLDB=Y;Locale=<ru>[;SchJobDn=Y][;SUsr=…;SPwd=…]`
— `Locale` на практике обязателен, `DBUID` тоже (иначе берётся пользователь ОС сервера). «Создана» и «была»
различаются кодом лишь отчасти: «была» — rc=1 и текст `Указанная информационная база уже существует.`, но rc=1
дают и все прочие отказы, поэтому без разбора прозы (запрещено правилом) решение строится на предварительной
проверке — `rac infobase summary list` / `infobase info --name` до запуска; она работает без учётных данных только
при пустом списке администраторов кластера. Наличие БД СУБД `CREATEINFOBASE` не различает: `CrSQLDB=Y` и `=N`
одинаково принимают существующую БД, включая БД с чужой ИБ. Сбой создания может оставить в СУБД брошенную БД, а
повтор — зарегистрировать ИБ на неё; превью должно об этом предупреждать. `/Out` нельзя показывать и хранить как
есть — в нём пароли.
