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
агент принимает пустой логин с пустым паролем, шлюз — пустой пароль только с пустым именем
(замер 06.10.2026 на 8.3.27.2074, «Сеансы автономного сервера через SSH-шлюз»; запись 13.09
о «любом имени» он не подтвердил). База с
пользователями: только пользователь ИБ и его пароль. Пользователя «по умолчанию» нет.

**Готовность зависит от среды, а не от конфига.** `ibcmd --pid` отвечает через 10–20 с
после старта автономного сервера; в одной из замеренных конфигураций — с
`--enable-extended-designer-features` — не ответил за десять минут. `ibcmd --pid config
export` пишет ноль файлов и не завершается. `ibcmd --remote=ssh://` без терминала работает,
если ключ хоста уже есть в `~/.ssh/known_hosts`; терминал нужен только для подтверждения
неизвестного ключа, без него `ibcmd` падает с SIGSEGV (замер 06.10.2026).

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

## Хранилище: случаи сомнения и признак подключения

Замер 06.10.2026. Задача: [#186](https://github.com/IngvarConsulting/v8-runner-rust/issues/186). Платформа 8.3.27.2074 (файловое хранилище, macOS) и 8.5.4.1878 (сервер хранилища в Docker).

Продолжение раздела «Частичная загрузка в базу под хранилищем». Проверены случаи, в
которых раннер сейчас переводит частичную загрузку в полную: новый объект, удаление
объекта, правка свойств корня (`Configuration.xml`).

**Стенд А (8.3.27.2074).** Файловая база из фикстуры `tests/fixtures/designer/configuration`,
файловое хранилище `<W>/repoA`, созданное из неё (`/ConfigurationRepositoryCreate`
подключает базу сразу). Пользователь `Админ`. Перед каждым сценарием база и хранилище
восстанавливались из снимка нужного состояния захватов. Контрольная база без хранилища —
копия той же базы до создания хранилища.

**Стенд Б (8.5.4.1878).** Клиент на Mac, своя файловая база, хранилище
`tcp://127.0.0.1:1542/v8rm_repo` на сервисе `repo` стенда Docker. Конфигуратор 8.5.4
запустился, отказа по лицензии не было. Хранилище из снимка не восстанавливалось:
захваты добавлялись по нарастающей, восстанавливалась только база.

Каталоги XML (копии фикстуры):

- `cfgAdd` — новый `Catalogs/Справочник2.xml` (копия `Справочник1` с новыми UUID) и строка
  `<Catalog>Справочник2</Catalog>` в `Configuration.xml`;
- `cfgDel` — нет `Enums/Перечисление1.xml` и строки `<Enum>Перечисление1</Enum>` в `Configuration.xml`;
- `cfgRoot` — в `Configuration.xml` изменён только `Comment` корня;
- `cfgCat` — изменён только `Comment` у `Справочник1` (для базовых случаев стенда Б).

Результат загрузки проверялся выгрузкой `/DumpConfigToFiles`: есть ли
`Catalogs/Справочник2.xml`, `Enums/Перечисление1.xml`, какой `Comment` у корня.
Контроль без хранилища: те же частичные загрузки `cfgAdd` и `cfgDel` в базу без
хранилища дают `rc=0`, объект добавляется и удаляется.

Списки захвата, которые принял `/ConfigurationRepositoryLock -objects`:

```xml
<!-- корень без подчинённых: «Объект захвачен для редактирования: Конфигурация» -->
<Objects xmlns="http://v8.1c.ru/8.3/config/objects" version="1.0">
	<Configuration includeChildObjects="false"/>
</Objects>
<!-- корень с подчинёнными: захвачены корень и все 45 объектов фикстуры -->
<Objects xmlns="http://v8.1c.ru/8.3/config/objects" version="1.0">
	<Configuration includeChildObjects="true"/>
</Objects>
<!-- удаляемый объект -->
<Objects xmlns="http://v8.1c.ru/8.3/config/objects" version="1.0">
	<Object fullName="Перечисление.Перечисление1" includeChildObjects="true"/>
</Objects>
```

### Случаи сомнения, 8.3.27.2074

Все загрузки — `/LoadConfigFromFiles <dir> -files "<список>" -partial` без параметров
хранилища, поэтому каждый `/Out` начинается строкой «Соединение с хранилищем
конфигурации не установлено»; в таблице она опущена.

| Захвачено | Загрузка | Код | `/Out` после первой строки | Состояние после |
| --- | --- | --- | --- | --- |
| ничего | добавление: `Configuration.xml,Catalogs/Справочник2.xml` | 1 | «Загрузка невозможна: объект метаданных Configuration не захвачен в хранилище!» | без изменений |
| корень без подчинённых | добавление: `Configuration.xml,Catalogs/Справочник2.xml` | 0 | пусто | `Справочник2` добавлен |
| корень без подчинённых | добавление только `Catalogs/Справочник2.xml` | 1 | «Ошибка добавления объекта Catalog uuid="…" : нельзя добавлять объекты метаданных без загрузки родительского объекта.» | без изменений |
| корень без подчинённых | `ibcmd infobase config import files --partial Configuration.xml Catalogs/Справочник2.xml` | 0 | «[INFO] Импорт файлов конфигурации из XML успешно завершен» | `Справочник2` добавлен |
| ничего | то же через `ibcmd` | 255 | «[ERROR] Импорт файлов конфигурации из XML завершен с ошибкой. Загрузка невозможна: объект метаданных Configuration не захвачен в хранилище!» | — |
| корень без подчинённых | удаление: `Configuration.xml` из `cfgDel` | 1 | «Загрузка невозможна: объект метаданных Enum.Перечисление1 не может быть удален!» / «Объект не захвачен в хранилище и/или не снят с поддержки.» | `Перечисление1` на месте |
| корень без подчинённых и `Перечисление1` | удаление: `Configuration.xml` из `cfgDel` | 0 | пусто | `Перечисление1` удалено |
| только `Перечисление1` | удаление: `Configuration.xml` из `cfgDel` | 1 | «Загрузка невозможна: объект метаданных Configuration не захвачен в хранилище!» | `Перечисление1` на месте |
| только `Перечисление1` | `-files "Enums/Перечисление1.xml"` из `cfgDel` (файла нет) | 1 | «Файл не обнаружен '<W>/cfgDel/Enums/Перечисление1.xml'. 2(0x00000002): No such file or directory, …» | — |
| ничего | правка корня: `Configuration.xml` из `cfgRoot` | 1 | «Загрузка невозможна: объект метаданных Configuration не захвачен в хранилище!» | без изменений |
| корень без подчинённых | правка корня: `Configuration.xml` из `cfgRoot` | 0 | пусто | `Comment` корня изменён |
| корень с подчинёнными | добавление | 0 | пусто | `Справочник2` добавлен |
| корень с подчинёнными | удаление | 0 | пусто | `Перечисление1` удалено |
| корень с подчинёнными | правка корня | 0 | пусто | `Comment` корня изменён |
| корень с подчинёнными | полная `/LoadConfigFromFiles <dir>` | 1 | «Загрузка невозможна: текущая конфигурация помещена в хранилище.» | — |
| корень с подчинёнными | `ibcmd infobase config import <dir>` | 255 | «[ERROR] Операция невозможна: информационная база подключена к хранилищу конфигурации» | — |

Итог по случаям:

1. **Добавление.** Проходит частичной загрузкой, если захвачен корень (`Configuration`),
   с `includeChildObjects` или без него. В список нужно включить и `Configuration.xml`,
   и файл нового объекта: один файл объекта отвергается («нельзя добавлять объекты
   метаданных без загрузки родительского объекта»). Без захвата корня отказ называет
   `Configuration`.
2. **Удаление.** Проходит частичной загрузкой одного `Configuration.xml`, если захвачены
   и корень, и удаляемый объект. Захват корня без захвата объекта даёт отдельный текст:
   «не может быть удален! Объект не захвачен в хранилище и/или не снят с поддержки».
   Захват объекта без корня отказывает по `Configuration`. Отказ атомарный: объект на месте.
3. **Правка свойств корня.** Проходит частичной загрузкой `Configuration.xml` при
   захваченном корне без подчинённых; без захвата отказ по `Configuration`.
4. Полная загрузка отказывает и тогда, когда захвачено всё (`includeChildObjects="true"`).

### Признак «база подключена к хранилищу», 8.3.27.2074

Сравнивались база под хранилищем (без захватов) и та же база без хранилища.

| Вызов | Под хранилищем | Без хранилища | Меняет базу |
| --- | --- | --- | --- |
| `ibcmd infobase config import <несуществующий каталог>` | 255, «[ERROR] Операция невозможна: информационная база подключена к хранилищу конфигурации» | 255, «[ERROR] Импорт конфигурации из XML завершен с ошибкой Файл не обнаружен '<W>/nonexistent_dir'» | нет: отказ до чтения каталога |
| `/LoadConfigFromFiles <несуществующий каталог>` | 1, «Файл не обнаружен '…'», строки о хранилище нет | 1, тот же текст | нет; не различает |
| `/DumpConfigToFiles <dir>` | 0, `/Out`: «Соединение с хранилищем конфигурации не установлено» | 0, `/Out` пустой | нет |
| выгрузка `/DumpConfigToFiles`, сравнение каталогов | — | — | `diff -r` пуст: в выгрузке (вместе с `ConfigDumpInfo.xml`) следа привязки нет |
| `/ConfigurationRepositoryReport <f> -ReportFormat txt` без параметров хранилища | 1, «Соединение с хранилищем конфигурации не установлено» (дважды), файла нет | 1, «Неклассифицированная ошибка работы с хранилищем конфигурации.», файла нет | нет |
| `/ConfigurationRepositoryUpdateCfg` без параметров хранилища | 1, «Соединение … не установлено» (дважды), «Ошибка обновления конфигурации из хранилища» | 1, «Неклассифицированная ошибка работы с хранилищем конфигурации.», «Ошибка обновления конфигурации из хранилища» | нет |
| `/ConfigurationRepositoryUpdateCfg` с параметрами хранилища | 0, «Обновление конфигурации из хранилища успешно завершено» (с рамкой «Начало/завершена операции с хранилищем») | 0, тот же итог без рамки; база после этого не подключена (проба `ibcmd` выше отвечает «Файл не обнаружен») | без хранилища — да: получает конфигурацию из хранилища |
| `/DumpDBCfgList` | 1, «Ошибка в параметрах командной строки.» | 1, тот же текст | — |
| `ibcmd help config`, `ibcmd help infobase` | команд хранилища конфигурации нет (8.3.27 и 8.5.4; в 8.5.4 добавились `config update` — обновление из файла — и `checksum`, к хранилищу не относятся) | — | — |
| файл базы `1Cv8.1CD` | путь хранилища, `repoA`, `Админ` открытым текстом (UTF-8, UTF-16LE) не найдены | — | — |

Кода выхода, который отличает базу под хранилищем, не нашлось: везде коды совпадают.
Различает только текст. Ближе всего к машинной проверке
`ibcmd infobase config import <несуществующий каталог>`: в базе под хранилищем она
отвечает фиксированной строкой `[ERROR] Операция невозможна: информационная база
подключена к хранилищу конфигурации` и ничего не меняет; в базе без хранилища падает на
отсутствующем каталоге и тоже ничего не меняет. Второй признак — первая строка `/Out`
у `/DumpConfigToFiles` без параметров хранилища.

### Что видно в хранилище после случаев 1–3

`/ConfigurationRepositoryCommit` не выполнялся. После добавления, удаления и правки
корня отчёт хранилища (`/ConfigurationRepositoryReport -ReportFormat txt`) показывает
одну версию — «Создание хранилища конфигурации», как до загрузок; отчёты до и после
отличаются только временем построения. Файлы хранилища после частичной загрузки
побайтно совпадают со снимком (`diff -rq` пуст); `1cv8ddb.1CD` меняется только от
самого подключения с отчётом. Частичная загрузка к хранилищу не обращается и в него
ничего не помещает: изменения остаются в конфигурации базы.

### Сервер хранилища, 8.5.4.1878

| Захвачено | Загрузка | Код | `/Out` после «Соединение … не установлено» |
| --- | --- | --- | --- |
| ничего | `-files "Catalogs/Справочник1.xml" -partial` (изменён) | 1 | «Загрузка невозможна: объект метаданных Catalog.Справочник1 не захвачен в хранилище!» |
| ничего | полная `/LoadConfigFromFiles <dir>` | 1 | «Загрузка невозможна: текущая конфигурация помещена в хранилище.» |
| ничего | добавление, удаление, правка корня (как выше) | 1 | каждое: «… объект метаданных Configuration не захвачен в хранилище!» |
| ничего | `ibcmd infobase config import <dir>` | 255 | «[ERROR] Операция невозможна: информационная база подключена к хранилищу конфигурации» |
| `Справочник1` | `-files "Catalogs/Справочник1.xml" -partial` | 0 | пусто |
| корень без подчинённых | добавление `Configuration.xml,Catalogs/Справочник2.xml` | 0 | пусто; `Справочник2` добавлен |
| корень без подчинённых | удаление (`Configuration.xml` из `cfgDel`) | 1 | «… Enum.Перечисление1 не может быть удален!» / «Объект не захвачен в хранилище и/или не снят с поддержки.» |
| корень и `Перечисление1` | удаление | 0 | пусто; `Перечисление1` удалено |
| корень и `Перечисление1` | правка корня | 0 | пусто; `Comment` изменён |
| корень и `Перечисление1` | `ibcmd … import files --partial Configuration.xml Catalogs/Справочник2.xml` | 0 | «[INFO] Импорт файлов конфигурации из XML успешно завершен» |
| корень с подчинёнными | полная `/LoadConfigFromFiles <dir>` | 1 | «Загрузка невозможна: текущая конфигурация помещена в хранилище.» |

Признак подключения на 8.5.4:

| Вызов | Под хранилищем | Без хранилища |
| --- | --- | --- |
| `ibcmd infobase config import <несуществующий каталог>` | 255, «[ERROR] Операция невозможна: информационная база подключена к хранилищу конфигурации» | 255, «[ERROR] Импорт конфигурации из XML завершен с ошибкой Файл не обнаружен '…'» |
| `/DumpConfigToFiles <dir>` | 0, «Соединение с хранилищем конфигурации не установлено» | 0, `/Out` пустой; выгрузки совпадают (`diff -r` пуст) |
| `/ConfigurationRepositoryReport <f>` без параметров хранилища | 1, «Соединение … не установлено» (дважды) | **не завершился**: процесс висел 3 мин 27 с без вывода, снят `kill` (`rc=143`, `/Out` пустой). Окна не смотрели; вероятно, модальный диалог |

Отчёт хранилища после всех загрузок — одна версия. **Поведение то же**, что у файлового
хранилища 8.3.27; отличие только в зависании отчёта без параметров на базе без
хранилища.

Время: Конфигуратор 8.3.27 — 2,6–3,8 с на вызов, `ibcmd` 8.3.27 — 5–6 с; Конфигуратор
8.5.4 — 11,2–11,6 с, `ibcmd` 8.5.4 — 22–23 с, создание серверного хранилища — 12,7 с.

Стенд Б после замера: каталог `/var/lib/onec/repo/v8rm_repo` в контейнере `repo` удалён,
`/var/lib/onec/repo` снова пуст. Хранилища `test` в этом каталоге не было ни до, ни после
замера: до создания `v8rm_repo` каталог был пуст, файлов `1cv8ddb.1CD` в контейнере не было.
Контейнеры, compose и база `dev` не трогались.

Сбой подготовки: первые попытки `/ConfigurationRepositoryCreate` возвращали `rc=1`
«Создание хранилища конфигурации завершилось с ошибкой», потому что zsh не разбивает
переменную на слова и параметры хранилища ушли одним аргументом. С отдельными
аргументами создание прошло; к поведению платформы это не относится.

```text
# 8.3.27.2074, стенд А
ibcmd infobase create --data=<d> --db-path=<ib>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /LoadConfigFromFiles <cfg> /UpdateDBCfg /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /ConfigurationRepositoryF <W>/repoA /ConfigurationRepositoryN Админ /ConfigurationRepositoryP <пароль> /ConfigurationRepositoryCreate /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /ConfigurationRepositoryF <W>/repoA /ConfigurationRepositoryN Админ /ConfigurationRepositoryP <пароль> /ConfigurationRepositoryLock -objects <lock_cfg_obj.xml> /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /LoadConfigFromFiles <W>/cfgAdd -files "Configuration.xml,Catalogs/Справочник2.xml" -partial /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /LoadConfigFromFiles <W>/cfgAdd -files "Catalogs/Справочник2.xml" -partial /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /LoadConfigFromFiles <W>/cfgDel -files "Configuration.xml" -partial /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /LoadConfigFromFiles <W>/cfgDel -files "Enums/Перечисление1.xml" -partial /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /LoadConfigFromFiles <W>/cfgRoot -files "Configuration.xml" -partial /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /LoadConfigFromFiles <W>/cfgRoot /Out <f>
ibcmd infobase config import files --data=<d> --db-path=<ib> --base-dir=<W>/cfgAdd --partial Configuration.xml Catalogs/Справочник2.xml
ibcmd infobase config import --data=<d> --db-path=<ib> <W>/cfgRoot
ibcmd infobase config import --data=<d> --db-path=<ib> <W>/nonexistent_dir
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /DumpConfigToFiles <dir> /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /LoadConfigFromFiles <W>/nonexistent_dir /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /ConfigurationRepositoryReport <f.txt> -ReportFormat txt /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /ConfigurationRepositoryUpdateCfg /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /ConfigurationRepositoryF <W>/repoA /ConfigurationRepositoryN Админ /ConfigurationRepositoryP <пароль> /ConfigurationRepositoryUpdateCfg /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /DumpDBCfgList /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /ConfigurationRepositoryF <W>/repoA /ConfigurationRepositoryN Админ /ConfigurationRepositoryP <пароль> /ConfigurationRepositoryReport <f.txt> -ReportFormat txt /Out <f>

# 8.5.4.1878, стенд Б: те же формы, хранилище на сервере
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /ConfigurationRepositoryF tcp://127.0.0.1:1542/v8rm_repo /ConfigurationRepositoryN Админ /ConfigurationRepositoryP <пароль> /ConfigurationRepositoryCreate /Out <f>
1cv8 DESIGNER /F <ib> /DisableStartupDialogs /ConfigurationRepositoryF tcp://127.0.0.1:1542/v8rm_repo /ConfigurationRepositoryN Админ /ConfigurationRepositoryP <пароль> /ConfigurationRepositoryLock -objects <lock.xml> /Out <f>
docker compose exec -T repo sh -c 'rm -rf /var/lib/onec/repo/v8rm_repo; ls -la /var/lib/onec/repo'
```

**Вывод для потребителей (#220).** До запуска раннер может узнать только одно: база
подключена к хранилищу. Надёжного кода выхода для этого нет. Без изменений базы признак
даёт `ibcmd infobase config import <несуществующий каталог>`: фиксированная строка
«[ERROR] Операция невозможна: информационная база подключена к хранилищу
конфигурации» против «Файл не обнаружен» у базы без хранилища, код 255 в обоих случаях.
Второй признак — первая строка `/Out` у `/DumpConfigToFiles`. Оба различаются только по
тексту. Захваты по-прежнему не читаются. Под хранилищем полная загрузка отказывает
всегда, даже при захвате всей конфигурации, поэтому переводить случай сомнения в полную
загрузку бесполезно. Все три случая выполнимы частичной загрузкой, если захвачено нужное:
новый объект — `Configuration.xml` вместе с файлом объекта при захваченном корне;
удаление — `Configuration.xml` без удалённого объекта при захваченных корне и самом
удаляемом объекте; правка корня — `Configuration.xml` при захваченном корне. Иначе
частичная загрузка отказывает атомарно (`rc=1` у Конфигуратора, `rc=255` у `ibcmd`) и
называет объект: `Configuration` при незахваченном корне или «не может быть удален» при
незахваченном удаляемом объекте. В хранилище при этом ничего не попадает. Серверное
хранилище 8.5.4.1878 ведёт себя так же, как файловое 8.3.27.2074.

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
| `Ref` уже зарегистрирован (любые `DB`, `CrSQLDB=Y` или `CrSQLDB=N`) | 1 | `Указанная информационная база уже существует.` | ничего; проверка идёт **до** аутентификации администратора |

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

## Отмена у чужого агента и шлюза SSH

**Замер п. 1 на 8.3.27 — раздел «Разрыв сессии с агентом во время долгой команды».** Задача: [#296](https://github.com/IngvarConsulting/v8-runner-rust/issues/296).
Правило, которое ждёт ответа:
[INV.USE-CASES.AN-ATTACHED-AGENT-ANSWERS-CANCELLED-AFTER-ITS-COMMAND-ENDS](../../spec/rules/use-cases/an-attached-agent-answers-cancelled-after-its-command-ends.md).

Вопрос: снимает ли команду, которая выполняется на стороне агента, то, что раннер делает
при отмене вне критической фазы, — `common disconnect-ib` в той же shell-сессии, а за ним
закрытие канала и соединения SSH. Агентом, которого запускает сам раннер, это не
решается: его процесс раннер гасит и ждёт.

**Стенд.** Выброшенная копия файловой базы с конфигурацией, выгрузка которой идёт не меньше
минуты. Отдельно на каждом пути:

- агент Конфигуратора (`attached`): `1cv8 DESIGNER /F <база> /AgentMode /AgentPort <порт>
  /AgentSSHHostKeyAuto /AgentBaseDir <каталог>`, платформа 8.3.27 и 8.5.1;
- шлюз автономного сервера (`gate`): `ibsrv` с той же базой и включённым шлюзом SSH,
  платформа 8.3.27 и 8.5.1.

Команда — долгая и только читающая, такая, какую раннер снимает: `infobase-tools dump-ib
<файл>.dt`, отдельным прогоном `config dump-config-to-files <каталог>`.

**Прогон.** Клиент SSH — раннер с журналом обмена или любой клиент с пустым логином.

1. Открыть сессию, `options set --output-format json`, `common connect-ib`.
2. Отправить долгую команду, дождаться, пока файл `.dt` (каталог выгрузки) начнёт расти.
3. Вариант А: в ту же сессию, не дожидаясь ответа, отправить `common disconnect-ib` и
   сразу закрыть канал и соединение — так делает раннер: отмена уже стоит, и ответа на
   `disconnect-ib` он не ждёт. Вариант Б: сразу закрыть
   соединение без `disconnect-ib`. Вариант В (контроль): не трогать сессию до ответа.
4. Каждую секунду до конца контрольного прогона записывать размер `.dt` (число файлов
   каталога), загрузку процессора процессом `1cv8` или `ibsrv` и — из новой сессии —
   исход `common connect-ib`.

**Что записать.** Когда пришёл ответ на `disconnect-ib` и был ли он (сразу или после конца
выгрузки), рос ли файл после разрыва, когда новая сессия смогла подключиться к базе, цел ли
итоговый `.dt` (`ibcmd infobase restore` в ещё одну выброшенную копию).

**Как отличить.** Команда снята, если в пределах нескольких секунд после разрыва файл
перестаёт расти, процесс перестаёт грузить процессор, новая сессия подключается сразу, а
`.dt` неполон или удалён. Команда продолжается, если файл растёт до размера контрольного
прогона, новая сессия не получает базу до конца выгрузки, а `.dt` восстанавливается.
Ответ на `disconnect-ib` только после конца выгрузки значит, что shell исполняет команды по
очереди и разрыв, а не `disconnect-ib`, решает исход. Во втором случае раннер по решению
владельца не рвёт сессию, а ждёт ответа агента на текущую команду.

## Версия формата файла версий

Замер 07.10.2026. Задача: [#403](https://github.com/IngvarConsulting/v8-runner-rust/issues/403). Платформы
8.3.27.2074 и 8.5.4.1878, macOS, файловые базы с фикстурой `tests/fixtures/designer/configuration`.
`<W>` — рабочий каталог замера. Версия формата живёт в двух местах: атрибут `version` корня
`ConfigDumpInfo.xml` и тот же атрибут у `MetaDataObject` каждого XML-файла выгрузки.

**Что пишут исполнители.**

| платформа | Конфигуратор `/DumpConfigToFiles` | `ibcmd config export` | агент `config dump-config-to-files` |
| --- | --- | --- | --- |
| 8.3.27.2074 | 2.20 | 2.20 | 2.20 |
| 8.5.4.1306, 8.5.4.1683, 8.5.4.1878 | 2.22 (1878) | 2.22 | не мерено |

Выгрузки Конфигуратора, `ibcmd` и агента одной базы побайтно равны (`diff -rq` пуст), включая
`ConfigDumpInfo.xml`. `ibcmd config export info` пишет тот же `ConfigDumpInfo.xml` (`--out` —
каталог, не файл: путь к файлу даёт rc=255 «Файл не обнаружен '<out>/ConfigDumpInfo.xml'»; без
`--out` — в stdout). На 8.5.1.x стенда Конфигуратора и `ibcmd` нет — только тонкий клиент.

**Выгрузка по изменившемуся.** В копию выгрузки подставлялась версия в `ConfigDumpInfo.xml`;
файлы выгрузки — той же платформы. «Перезаписано» — файлы с новым временем изменения.

| версия в файле | Конфигуратор `-update` | `-update -force` | `ibcmd export --sync` | `--sync --force` | `-getChanges` / `export status` |
| --- | --- | --- | --- | --- | --- |
| своя (2.20 на 8.3.27, 2.22 на 8.5.4) | rc=0, обычная | — | rc=0 | — | обычный ответ |
| старше своей (2.10, 2.17, 2.19 на 8.3.27; 2.20, 2.21 на 8.5.4) | **rc=101**, ничего не перезаписано | rc=0, полная: все файлы, файл версий получает свою версию | rc=255 | rc=0, полная | **rc=0, пусто — «изменений нет»** |
| атрибута `version` нет | rc=101 | rc=0, полная | rc=255 | rc=0, полная | rc=0, пусто |
| новее своей (2.21, 2.99, 3.0 на 8.3.27; 2.23 на 8.5.4; настоящая выгрузка 8.5.4 на 8.3.27) | rc=1 | rc=1, `-force` не помогает | rc=255 | rc=255 | rc=1 / rc=255 |

Тексты. Старше: Конфигуратор — «Обновление XML выгрузки невозможно: версия формата платформы
отличается от версии формата выгрузки.» / «Для синхронизации необходимо выполнить полную
выгрузку.»; `ibcmd` — «Версия формат выгрузки и платформы не совпадают. Требуется
экспортировать конфигурацию полностью.». Новее: все — «Неизвестная версия формата <v>
загружаемого файла <путь>/ConfigDumpInfo.xml». Агент 8.3.27 отвечает теми же текстами, но оба
случая у него одного рода — `ConfigFilesError`; `--update --force` на старшей версии делает
полную выгрузку.

Главное: **код 101 у Конфигуратора — структурный признак «формат выгрузки старше платформы»**,
отличный от rc=1 «неизвестная, новее». У `ibcmd` оба случая — rc=255, различает их только проза.
**Прогноз не видит чужую старую версию**: `-getChanges` и `config export status` по файлу 2.17
отвечают «изменений нет», а следующий `-update` отказывает. Запись раздела «`-getChanges`» о
версии 2.17 («принят, обычный список») верна только для `-getChanges`: сам `-update` такой файл
не принимает.

**Загрузка из каталога с чужой версией** (`/LoadConfigFromFiles`, `-updateConfigDumpInfo`,
`ibcmd config import`; одинаково):

| вход | исход |
| --- | --- |
| версия новее только в `ConfigDumpInfo.xml` (2.21 на 8.3.27) | rc=0, загружено: версию файла версий загрузка не проверяет |
| версия новее во всех XML (2.21, 3.0; настоящая выгрузка 8.5.4 на 8.3.27) | Конфигуратор rc=1, `ibcmd` rc=255: «Неизвестная версия формата 2.22 загружаемого файла <путь>/Configuration.xml» |
| версия старше во всех XML (2.17 на 8.3.27; выгрузка 8.3.27 на 8.5.4) | rc=0, загружено |
| `ConfigDumpInfo.xml` нет | rc=0 |

`-updateConfigDumpInfo` переписывает `ConfigDumpInfo.xml` **в исходном каталоге** со своей
версией (2.21 → 2.20); без этого ключа загрузка каталог не трогает, `ibcmd config import`
тоже.

**`ibcmd config export` без `--sync`** (п. 4 задачи):

| каталог | без ключей | `--force` |
| --- | --- | --- |
| не существует или пуст | rc=0, полная выгрузка, каталог создаётся | так же |
| прежняя выгрузка или любые посторонние файлы | **rc=255, «Каталог <путь> не пуст.»**, ничего не тронуто | rc=255, то же: `--force` без `--sync` непустой каталог не принимает |

Конфигуратор `/DumpConfigToFiles` без `-update` в каталог с посторонними файлами выгружает
поверх, rc=0, посторонние файлы остаются.

**`ibcmd config export --sync`** (п. 5 задачи):

| каталог | `--sync` | `--sync --force` |
| --- | --- | --- |
| своя выгрузка, изменений нет | rc=0, ничего не перезаписано, посторонние файлы целы | rc=0, то же |
| нет `ConfigDumpInfo.xml` (каталог пуст, с выгрузкой без файла версий, с посторонними файлами) | rc=255, «В каталоге выгрузки отсутствует файл <путь>/ConfigDumpInfo.xml. Требуется экспортировать конфигурацию полностью.» | не мерено |
| каталога нет | rc=255, «Каталог <путь> не существует.» | не мерено |
| удалён объект в конфигурации (`export status` — `modified: all`) | **rc=255, «Требуется экспортировать конфигурацию полностью.»** | rc=0, полная |
| версия формата старше | rc=255 (см. выше) | rc=0, полная |

**`--sync --force`, переходя на полную выгрузку, очищает каталог целиком**: исчезают
посторонние файлы, скрытые каталоги, `.git` со всем содержимым, `.gitignore`, `README.md`.
Проверено и на старшей версии формата, и на удалении объекта. Конфигуратор в тех же случаях
(`-update` при `FullDump`, `-update -force` при старшей версии) переписывает только файлы
выгрузки и удаляет файлы удалённых объектов; `.git`, `.gitignore`, `README.md`, скрытые
каталоги и посторонние файлы остаются.

```text
1cv8 DESIGNER /F <W>/ib /DisableStartupDialogs /DisableStartupMessages /DumpConfigToFiles <W>/v/D2.17 -update [-force] /Out …
1cv8 DESIGNER /F <W>/ib … /DumpConfigToFiles <W>/v/Dg2.17 -update -getChanges <W>/out/ch.txt /Out …
1cv8 DESIGNER /F <W>/ibL … /LoadConfigFromFiles <W>/v/L_2.21_all [-updateConfigDumpInfo] /Out …
ibcmd config export --db-path=<W>/ib [--force] <W>/x/e_dump
ibcmd config export --sync [--force] --db-path=<W>/ib <W>/v/I2.17
ibcmd config export status --db-path=<W>/ib --base=<W>/v/Is2.17/ConfigDumpInfo.xml
ibcmd config export info --db-path=<W>/ib --out=<W>/infod
ibcmd config import --db-path=<W>/ibL <W>/v/L_2.21_all
```

**Вывод для потребителей.**

- Таблица «платформа → версия формата» (#214, `src/platform/dump_format.rs`): 8.3.27 → 2.20,
  8.5.4 → 2.22. Выгрузка по изменившемуся принимает **только свою** версию: старшая даёт отказ
  (rc=101 / rc=255), а не полную выгрузку; полную даёт только `-force` / `--sync --force`.
- Отказ загрузки по формату новее определяется версией в XML-файлах (`Configuration.xml`), а
  не в `ConfigDumpInfo.xml`: каталог с новой версией только в файле версий платформа
  загружает. Сверка раннера по `ConfigDumpInfo.xml` строже платформы, а без файла версий её
  нечем делать.
- Прогноз режима (#166) надо дополнять сверкой версии: по чужой старой версии
  `-getChanges` и `export status` отвечают «изменений нет».
- `ibcmd config export` без `--sync` в непустой каталог **всегда** отказывает: выгрузка
  `ibcmd` поверх каталога без файла версий (`config_export_over`, #217) так не работает;
  полная выгрузка `ibcmd` годится только в пустой каталог (как `config_export_full` в
  промежуточный).
- `ibcmd --sync` после удаления объекта отказывает, а не переходит на полную выгрузку, как
  Конфигуратор.
- `ibcmd config export --sync --force` в рабочем дереве уничтожает репозиторий. Раннер этого
  сочетания сейчас не строит; строить его по каталогу пользователя нельзя.

## Прогноз режима выгрузки: язык и словарь `export status`

Замер 07.10.2026. Задача: [#166](https://github.com/IngvarConsulting/v8-runner-rust/issues/166).
Продолжение раздела «Список изменений выгрузки: `-getChanges`». Сценарии на копиях файловой
базы: правка модуля (частичная загрузка `Module.bsl`), новый справочник (частичная загрузка
`Configuration.xml` и его XML), удалённый объект `Бот1`.

**`-getChanges` от языка не зависит.** Файл при `/L ru`, `/L en` и `/L de` побайтно один и тот
же в каждом сценарии (8.3.27): `New: Catalog.Справочник9`, `Modified: CommonModule.ОбщийМодуль1`,
`FullDump`, пусто (только BOM). Имена классов метаданных — английские, имена объектов — как в
конфигурации.

**Словарь `ibcmd config export status`** — одинаковый на 8.3.27.2074 и 8.5.4.1878, от `LANG`
и `LC_ALL` не зависит (побайтно):

| изменение | полная форма | `--short` |
| --- | --- | --- |
| новый объект | `added: Catalog.Справочник9` | `A: Catalog.Справочник9` |
| изменённый объект | `modified: CommonModule.ОбщийМодуль1` | `M: CommonModule.ОбщийМодуль1` |
| удалён объект (у Конфигуратора `FullDump`) | `modified: all` | `M: all` |
| изменений нет | пусто | пусто |

Ключ отделён от значения двоеточием и пробелом; записи `deleted` не встретилось. Код возврата
во всех случаях 0. С `--out` файл в UTF-8 с BOM и CRLF; без `--out` тот же список идёт в
stdout без BOM, строки через LF, stderr пуст. Порядок: сначала `added`, затем `modified` по
имени — как у `-getChanges`.

```text
1cv8 DESIGNER /F <W>/ibM /DisableStartupDialogs /DisableStartupMessages /L en /DumpConfigToFiles <W>/d0 -getChanges <W>/out/gc.txt /Out …
ibcmd config export status --db-path=<W>/ibM --base=<W>/d0/ConfigDumpInfo.xml [--short] [--out=<W>/out/st.txt]
```

**Вывод для потребителей.** Прогноз разбирается по ключу до двоеточия: `FullDump` у
Конфигуратора и значение `all` у ключа `modified` у `ibcmd` — «полная»; `New`/`Modified` и
`added`/`modified` — «по изменившемуся». Язык Конфигуратора закреплять не нужно. Версию формата
файла версий прогноз не проверяет (см. «Версия формата файла версий»).

## Сборка `make` во временной базе

Замер 07.10.2026. Задача: [#416](https://github.com/IngvarConsulting/v8-runner-rust/issues/416).
Платформа 8.3.27.2074, macOS. Командные строки — в той форме, которую строят
`src/use_cases/throwaway_infobase.rs` и `src/platform/{designer,ibcmd}.rs`. Каждый пакет
проверен: загружен `/LoadCfg` в чистую базу (`.cfe` — `-Extension Расширение1` в базу с
конфигурацией), выгружен `/DumpConfigToFiles` и сравнен с исходниками.

| сочетание | исход |
| --- | --- |
| 1. `CREATEINFOBASE File='<ib>'`, затем `DESIGNER /IBConnectionString File=<ib> /LoadConfigFromFiles <xml>` (без `-updateConfigDumpInfo` и `/UpdateDBCfg`), `/DumpCfg <f>.cf`; затем расширение `/LoadConfigFromFiles <xml_cfe> -Extension Расширение1`, `/DumpCfg <f>.cfe -Extension Расширение1` | все rc=0; `.cf` 114 987 байт, `.cfe` 6 164; содержимое равно исходникам |
| 2. `ibcmd infobase --data <d> --db-path <ib> create`, затем `ibcmd infobase --data <d> --db-path <ib> config import --out=<f> <xml>` для `.cf` и `.cfe` | все rc=0; `.cf` 116 029 байт, `.cfe` 6 824; содержимое равно исходникам; `<d>` получает `ipc-data log-data perf-data session-data temp users-data` |
| 3а. `/LoadExternalDataProcessorOrReportFromFiles <root.xml> <f>.epf` (и `.erf`) в базе из п. 1: конфигурация загружена, не применена | rc=0 |
| 3б. то же для обработки с реквизитом типа `CatalogRef.Справочник1` | rc=0; реквизит с этим типом есть в пакете (выгружен обратно `/DumpExternalDataProcessorOrReportToFiles`) |
| 3в. обработка с `CatalogRef.Справочник1` в пустой базе, без конфигурации | **rc=1**, «Неизвестное имя типа - CatalogRef.Справочник1» / «Ошибка загрузки документа.»; обработка без ссылочных типов там же — rc=0 |
| 4. Конфигуратор в базе, созданной `ibcmd infobase --data <d> --db-path <ib> create`: `/LoadConfigFromFiles`, `/DumpCfg`, расширение, внешняя обработка с `CatalogRef` | все rc=0; `.cf` по содержимому равен выгрузке; после Конфигуратора `ibcmd config import --out` в той же базе — rc=0 |

`CREATEINFOBASE` без `Locale` создаёт базу в локали системы: в протоколе
`File='…';Locale = "en_AU";`. Пакеты обоих путей не детерминированы по байтам (размеры
плавают), равенство — только по выгрузке, как в разделе «Сборка пакета из XML».

```text
1cv8 CREATEINFOBASE "File='<W>/m/ib1'" /Out …
1cv8 DESIGNER /DisableStartupDialogs /DisableStartupMessages /IBConnectionString File=<W>/m/ib1 /LoadConfigFromFiles <W>/src_cfg /Out …
1cv8 DESIGNER … /IBConnectionString File=<W>/m/ib1 /DumpCfg <W>/m/out/d.cf /Out …
1cv8 DESIGNER … /IBConnectionString File=<W>/m/ib1 /LoadExternalDataProcessorOrReportFromFiles <W>/m/extref/ВнешняяОбработка1.xml <W>/m/out/pref.epf /Out …
ibcmd infobase --data <W>/m/data2 --db-path <W>/m/ib2 create
ibcmd infobase --data <W>/m/data2 --db-path <W>/m/ib2 config import --out=<W>/m/out/i.cf <W>/src_cfg
```

**Вывод для потребителей.** Все четыре сочетания #416 подтверждены. Внешней обработке со
ссылками на объекты конфигурации основная конфигурация в базе нужна, применять её не нужно.
Конфигуратор работает в базе, созданной `ibcmd`, поэтому внешние наборы можно собирать и в
ней.

## Признаки «есть непринятое» и «обновлено динамически»

Замер 07.10.2026. Задача: [#412](https://github.com/IngvarConsulting/v8-runner-rust/issues/412).
Платформа 8.3.27.2074 (признак непринятого — и 8.5.4.1878), macOS, копии файловой базы.

**Проба изнутри.** Внешняя обработка, запущенная `1cv8 ENTERPRISE /F <ib> /Execute <probe>.epf`,
пишет в файл значения `КонфигурацияИзменена()` и
`КонфигурацияБазыДанныхИзмененаДинамически()` и завершает сеанс (около 7 с на запуск).
**Снаружи** — `ibcmd infobase --data <d> --db-path <ib> config save <f>` против `config save
--db <f>` (около 6 с на вызов). `config save` детерминирован: два вызова дают одни и те же
байты, `--db` тоже, а Конфигуратор `/DumpCfg` и `/DumpDBCfg` дают те же байты, что `ibcmd`.

| состояние базы | `КонфигурацияИзменена()` | `…ИзмененаДинамически()` в новом сеансе | `save` = `save --db` |
| --- | --- | --- | --- |
| применена (`/UpdateDBCfg`) | false | false | да |
| частичная загрузка модуля без применения | true | false | нет |
| полная загрузка того же дерева без применения | true | false | нет |
| правка модуля и обратная правка, без применения | true | false | нет |
| непринятое, затем `ibcmd config reset` | false | false | да |
| `ibcmd config apply --dynamic=force` | false | **false** | да |
| динамическое применение, затем новое непринятое | true | false | нет |
| динамическое, затем `apply --dynamic=disable` | false | false | да |
| `/UpdateDBCfg -Dynamic+` и `-Dynamic-` | false | false | да |

На 8.5.4: принятая база — три `save` (два основных, один `--db`) побайтно равны; после
частичной загрузки без применения основная отличается, `--db` прежняя.

**Динамическое обновление снаружи не видно.** `КонфигурацияБазыДанныхИзмененаДинамически()`
относится к сеансу: сеанс, открытый до `ibcmd config apply --dynamic=force` (применение при
открытом сеансе файловой базы — rc=0), через 25 с получает `true`, а любой новый сеанс —
`false`. Идентификатор поколения после динамического и обычного применения одного вида (32
hex и `00000000`), сохранённые `.cf` основной и `--db` после динамического применения равны.
Постоянного признака «база обновлена динамически» в базе не нашлось ни у одного исполнителя.
Агент не мерен.

```text
1cv8 ENTERPRISE /F <W>/q/s1 /DisableStartupDialogs /DisableStartupMessages /Execute <W>/q/probe.epf
ibcmd infobase --data <W>/q/data --db-path <W>/q/s1 config save <W>/q/s1m.cf
ibcmd infobase --data <W>/q/data --db-path <W>/q/s1 config save --db <W>/q/s1d.cf
ibcmd infobase --data <W>/q/data --db-path <W>/q/ib config apply --force --dynamic=force
1cv8 DESIGNER /F <W>/q/s0 … /DumpCfg <W>/q/s0_dc1.cf | /DumpDBCfg <W>/q/s0_ddb1.cf
```

**Вывод для потребителей.** «Есть непринятое» (#216, `status --deep`) — структурный признак
есть: побайтное неравенство `config save` и `config save --db` (`/DumpCfg` и `/DumpDBCfg` у
Конфигуратора). Во всех девяти состояниях он совпал с `КонфигурацияИзменена()`, включая
загрузку того же содержимого — это «записано, но не применено», а не «отличается по смыслу».
Цена — два сохранения базы. «Обновлено динамически» как состояние базы платформа наружу не
отдаёт: признак есть только внутри сеанса, начатого до обновления.

## `ibcmd-rs` 0.4.0: сборка и разборка пакетов без платформы

Замер 07.10.2026. Задача: [#413](https://github.com/IngvarConsulting/v8-runner-rust/issues/413).
Выпуск `v0.4.0` из `github.com/Untru/ibcmd-rs`, архив `ibcmd-rs-0.4.0-x86_64-unknown-linux-gnu.zip`
(SHA-256 `f9b892cb…888db5` совпал с опубликованным). Запуск — контейнер `ubuntu:24.04`
linux/amd64 под эмуляцией на macOS, без сети. Входы — выгрузка 8.3.27.2074 той же фикстуры
`tests/fixtures/designer/*` и пакеты, собранные платформой (раздел «Сборка `make` во временной
базе»).

**Распространение.** Сборки есть только для Windows x64 и Linux x64; для macOS и arm64 сборок
нет. Linux-сборке нужна glibc ≥ 2.39: в Ubuntu 22.04 (glibc 2.35, образы стенда) она не
запускается — «version `GLIBC_2.39' not found». **Лицензии нет**: в репозитории нет файла
лицензии, GitHub лицензию не определяет (404), у `Cargo.toml` и SBOM выпуска поле лицензии
пусто. Сам README называет проект экспериментом до версии 1.0.

**CLI.** `ibcmd-rs --version` → `ibcmd-rs 0.4.0`. Сборка `.cf` из XML —
`cf bootstrap [--platform <8.3.27|8.5.1|сборка>] <каталог> <файл>`; существующий файл не
перезаписывается. Разборка — `cf export [--platform …] <файл> <каталог>`, тип пакета
определяется сам. Ответ — JSON в stdout при успехе и в stderr при отказе (`ok`, `errors[]` с
`code`, `message`, `element`). Коды: 0 — успех, 2 — отказ сборки и ошибка командной строки
(clap); код 1 «не поддерживается» в замере не встретился. Ключа `--base-free`, описанного в
README и `docs/COMMANDS.md` ветки `master`, у выпуска 0.4.0 нет: «unexpected argument
'--base-free'», rc=2.

**Сборка `.cf` из XML не работает на фикстуре.** `cf bootstrap` на дереве, которое выгрузила
сама платформа 8.3.27.2074 (`--platform` не задан, `8.3.27` или `8.3.27.2214` — одинаково),
отказывает с rc=2 до записи файла. Ниже — объекты, на которых он отказывал, если по очереди
убирать каждый следующий:

| объект | отказ |
| --- | --- |
| регистры бухгалтерии, накопления и расчёта, бизнес-процесс, справочник, планы счетов, видов расчёта и видов характеристик | `bootstrap_compile_failed`: `InvalidEnvelope("business object property inventory is not exact")` |
| бот, общий реквизит, общая форма, общий макет | `uses unsupported family \`Bot\`` (`CommonAttribute`, `CommonForm`, `CommonTemplate`) |
| `Configuration.xml` после удаления объектов | `Missing("uuid")` |

Обычный справочник из выгрузки 8.3.27.2074 не собирается. `.cfe` и `.epf` тем же
`bootstrap` не мерили: сборка основной конфигурации не прошла.

**Разборка пакета в XML работает частично.** `cf export` пакетов, собранных платформой:

| пакет | rc | итог |
| --- | --- | --- |
| `.cfe` (`Расширение1`) | 0 | дерево равно исходникам, кроме `ConfigDumpInfo.xml` |
| `.epf`, в том числе с реквизитом `CatalogRef.Справочник1`; `.erf` | 0 | побайтно равно выгрузке платформы `/DumpExternalDataProcessorOrReportToFiles` |
| `.cf` фикстуры | **0, `ok: true`** | 47 файлов из 51: нет `ChartsOfAccounts`, `ChartsOfCalculationTypes`, `FilterCriteria`, `WebSocketClients`; в `Configuration.xml` нет `ChildObjects`; ещё 7 файлов отличаются. Пропуски видны только в `export.storage.entries[]` (`disposition: "opaque"`, `message: "… not written …"`; итог `supported: 46, opaque: 7, failed: 0`). Ни код возврата, ни `ok`, ни `errors` о них не говорят |

```text
docker run --rm --platform linux/amd64 --network none -v <S>:/s ubuntu:24.04 /s/irs/x/ibcmd-rs-0.4.0-x86_64-unknown-linux-gnu/ibcmd-rs …
ibcmd-rs cf bootstrap [--platform 8.3.27] /s/w/d0 /s/irs/out/b.cf        # rc=2
ibcmd-rs cf bootstrap --base-free --platform 8.3.27 /s/w/d0 /s/irs/out/b.cf   # rc=2, нет ключа
ibcmd-rs cf export --platform 8.3.27 /s/irs/out/d.cf /s/irs/out/x_d.cf    # rc=0, 7 элементов opaque
ibcmd-rs cf export --platform 8.3.27 /s/irs/out/p.epf /s/irs/out/x_p.epf  # rc=0, равно платформе
```

**Вывод для потребителей.** Адаптер `make` через `ibcmd-rs` (#413) на выпуске 0.4.0 строить не
на чем: сборка `.cf` из XML отказывает на обычной конфигурации 8.3.27.2074, ключ `--base-free`
не выпущен. В `convert` пакет → XML (#236) годятся `.cfe`, `.epf` и `.erf`. Разборку `.cf`
раннеру без своей проверки полноты принимать нельзя: неполное дерево приходит с rc=0 и
`ok: true`, пропуски названы только в `opaque`. Без лицензии `tools download ibcmd-rs` и
распространение исключены; остаётся путь к утилите из настроек. Windows и macOS не мерены:
macOS-сборки нет, Windows на стенде нет.

**Перепроверка на `master` ibcmd-rs** (`e382f21`, 02.10.2026; собран `cargo build --release` на
macOS arm64 за 2 мин; сам называет себя `0.4.0`). `cf bootstrap --base-free --platform 8.3.27`:

| вход | исход |
| --- | --- |
| выгрузка фикстуры 8.3.27.2074 целиком | rc=2, `base_free_compile_failed`, четыре строки: `WebSocketClient` не знает ни `Configuration.xml`, ни сам объект; «no base-free compiler for ExternalDataSource yet»; право роли `ExclusiveModeTerminationAtSessionStart` — «unknown right» |
| то же без WebSocket-клиента, внешнего источника данных и этого права | rc=0. `.cf` платформа загружает (`/LoadCfg`) и применяет (`/UpdateDBCfg`), её выгрузка совпадает с деревом файл в файл |
| XML расширения | rc=0, `ok: true`, но `/LoadCfg -Extension` отвечает rc=1 «Ожидается файл расширения конфигурации». У пакета `storage_version` 5 и корень `root`/`version`/`versions`; у `.cfe` Конфигуратора — 6 и `configinfo` |
| XML внешней обработки | rc=2, ищет `Configuration.xml`: сборки `.epf`/`.erf` из XML нет |

`cf export` на `master` даёт то же, что 0.4.0: `.cf` неполон при `ok: true`, `.cfe`, `.epf` и
`.erf` выгружаются верно. Задачи автору — Untru/ibcmd-rs#432–#442; решение владельца —
встраивание ждёт выпуска с исправлениями (#413).

## Разбор пакета `convert` во временной базе

Замер 08.10.2026. Задача: [#416](https://github.com/IngvarConsulting/v8-runner-rust/issues/416)
(правило `INV.USE-CASES.CONVERT-SEQUENCES-ARE-MEASURED-ON-A-LIVE-PLATFORM`). Платформа
8.3.27.2074, macOS. Командная строка — в форме `IbcmdDsl::config_export_file`:

```text
ibcmd infobase --data <W>/cv/d --db-path <W>/cv/ib create                                    # rc=0
ibcmd config --data <W>/cv/d --db-path <W>/cv/ib export --file=<пакет> <каталог>              # rc=0, ~4,6 с
```

| пакет | исход |
| --- | --- |
| `.cf`, собранный Конфигуратором (`/DumpCfg`) или `ibcmd config import --out` | rc=0; XML по содержимому равен выгрузке Конфигуратора той же конфигурации (`diff -rq` без `ConfigDumpInfo.xml` пуст); `ConfigDumpInfo.xml` версии 2.20 отличается только `configVersion` |
| `.cfe` (`Расширение1`) обоих сборщиков | rc=0; XML равен исходникам расширения; **`ConfigDumpInfo.xml` не пишется** |

База после разбора остаётся пустой: `config generation-id` — сорок нулей. В stdout —
`[INFO] Экспорт конфигурации в XML...` / `…успешно завершен`, stderr пуст.

**Вывод для потребителей.** Форма, которую строит раннер, работает на свежей пустой базе
для обоих видов пакета. XML расширения, разобранный из `.cfe`, приходит без файла версий:
выгрузка по изменившемуся по нему невозможна, следующая выгрузка этого набора будет полной.

## Прогноз режима у расширения и у агента

Замер 08.10.2026. Задача: [#166](https://github.com/IngvarConsulting/v8-runner-rust/issues/166). Платформа
8.3.27.2074, macOS; продолжение раздела «Прогноз режима выгрузки: язык и словарь».

**Расширение** (`Расширение1`, загружено и применено): правка свойства параметра сеанса
частичной загрузкой, затем удаление этого объекта из `Configuration.xml` расширения.

| вызов | без изменений | правка объекта | удаление объекта |
| --- | --- | --- | --- |
| `/DumpConfigToFiles <dir> -Extension Расширение1 -update -getChanges <f>` | rc=0, пусто (BOM) | `Modified: SessionParameter.Расш1_ПараметрСеанса1` | `FullDump` |
| `ibcmd config export status --extension=Расширение1 --base=<dir>/ConfigDumpInfo.xml` | rc=0, пусто | `modified: SessionParameter.Расш1_ПараметрСеанса1` | `modified: all` |
| `ibcmd config export --extension=Расширение1 --sync <dir>` после удаления | | | **rc=255**, «Требуется экспортировать конфигурацию полностью.» |

Словарь тот же, что у основной конфигурации; отказ `--sync` после удаления (#424) касается и
расширений.

**Агент** (`1cv8 DESIGNER /AgentMode`, сессия `options set --output-format json`,
`common connect-ib`). Пути — относительно каталога пользователя агента:

| команда | ответ | файл списка |
| --- | --- | --- |
| `config dump-config-to-files --dir dA --update --get-changes chA.txt` (правка модуля) | `{"type": "success"}` | `Modified: CommonModule.ОбщийМодуль1` / `Modified: CommonModule.ОбщийМодуль1.Module` |
| то же без `--update` | `success` | тот же список |
| то же, удалён объект | `success` | `FullDump` |
| `--dir nodir --update --get-changes chN.txt` (каталога нет) | `{"type": "error", "error-type": "ConfigFilesError", "message": "Не удалось найти файл версий - …"}` | не создаётся |

С `--get-changes` агент, как и пакетный Конфигуратор, ничего не выгружает: время изменения
файлов каталога прежнее. Формат файла списка тот же: UTF-8 с BOM, CRLF, `New:`/`Modified:`/`FullDump`.

**Вывод для потребителей.** Прогноз режима доступен у всех трёх исполнителей — Конфигуратора,
агента и `ibcmd` — и для основной конфигурации, и для расширения, одним словарём на
исполнителя.

## Признак непринятого у расширения

Замер 08.10.2026. Задача: [#412](https://github.com/IngvarConsulting/v8-runner-rust/issues/412).
Платформа 8.3.27.2074, macOS; продолжение раздела «Признаки „есть непринятое“ и „обновлено
динамически“». Расширение `Расширение1` в файловой базе.

| состояние | `ibcmd config save --extension` ×2 | `… --db` | `/DumpCfg -Extension` ×2 | `/DumpDBCfg -Extension` |
| --- | --- | --- | --- | --- |
| применено | равны между собой и с `--db` | — | равны между собой и с `/DumpDBCfg` | — |
| полная загрузка расширения без `/UpdateDBCfg` | **разные при каждом вызове** | прежнее | **разные при каждом вызове** | прежнее |
| после `/UpdateDBCfg -Extension` | снова равны между собой и с `--db` | | равны | |

Сохранение непринятого расширения не детерминировано, но всегда отличается от сохранения
конфигурации базы данных; у применённого обе стороны равны побайтно. Признак «сохранение ≠
сохранение `--db`» поэтому держится и для расширения.

## Выгрузка агента против выгрузки Конфигуратора

Замер 08.10.2026. Задача: [#420](https://github.com/IngvarConsulting/v8-runner-rust/issues/420). Платформа
8.3.27.2074, macOS 27.0 (arm64). Файловая база с фикстурой `tests/fixtures/designer/configuration`
(`/LoadConfigFromFiles … /UpdateDBCfg`). Агент и пакетный Конфигуратор работали с одной базой
по очереди: агент закрыт до пакетного запуска. `<W>` — рабочий каталог замера, `<U>` —
каталог пользователя агента (`<W>/abase/0` по `agentbasedir.json`).

```text
1cv8 DESIGNER /IBConnectionString File=<W>/ib /AgentMode /AgentPort <свободный> /AgentListenAddress 127.0.0.1 /AgentSSHHostKey <W>/key /AgentBaseDir <W>/abase
  options set --output-format json
  common connect-ib
  config dump-config-to-files --dir=adump
  config dump-cfg --file=a.cf
1cv8 DESIGNER /F <W>/ib /DisableStartupDialogs /DisableStartupMessages /DumpConfigToFiles <W>/bdump /Out …
1cv8 DESIGNER /F <W>/ib /DisableStartupDialogs /DisableStartupMessages /DumpCfg <W>/b.cf /Out …
diff -rq <U>/adump <W>/bdump
cmp <U>/a.cf <W>/b.cf
```

| что сравнивали | 8.3.27.2074 |
| --- | --- |
| `config dump-config-to-files` и `/DumpConfigToFiles`, фикстура | 51 и 51 файл, `diff -rq` пуст, включая `ConfigDumpInfo.xml` |
| то же, большая конфигурация (фикстура и 10 000 общих модулей, 20 051 файл; три выгрузки агента из раздела о разрыве сессии) | `diff -rq` пуст |
| `config dump-cfg` и `/DumpCfg` | оба по 114 999 байт, `cmp` без расхождений: пакеты равны побайтно |

**Список расхождений пуст.** Раздел «Версия формата файла версий» уже записал равенство
выгрузок файлов на 8.3.27; этот замер добавляет пакет `.cf` и большую конфигурацию.
Агенту 8.5.4.1878 пакетный Конфигуратор не нашёл лицензии («Не найдена лицензия. Не обнаружен
ключ защиты программы или полученная программная лицензия!»), агент 8.5.4 при этом отвергал
вход по SSH; сравнение на 8.5.4 не мерено.

Попутно к п. 3 задачи: в этих сессиях после `options set --output-format json` каждый ответ
пришёл JSON-массивом. Отказ разбора (`config dump-cfg` без `--file`) — один массив: сообщения
`log` со справкой и последним `{"type": "error", "error-type": "CommandFormatError",
"message": "Неверный формат команды:Ошибка разбора параметра: file"}`. За 81 с выгрузки
20 051 файла до массива ответа агент не прислал ни байта. Вне массивов — только баннер
`1C:Enterprise 8.3 1C Designer Shell …` и приглашение `designer> ` после каждого массива, пока
его не выключили `--show-prompt=no`. Что агент печатает при разрыве сессии, клиент увидеть не
может; п. 3 этим не закрыт.

## Управляемый агент: одноразовый ключ раннера и свободный порт

Замер 08.10.2026. Задача: [#429](https://github.com/IngvarConsulting/v8-runner-rust/issues/429). Платформы
8.3.27.2074 и 8.5.4.1878, macOS 27.0 (arm64). Linux и Windows — не мерено: стенда нет; права
файла ключа на Windows (п. 4) тоже не мерены.

**Ключ и порт раннера.** Раннер, собранный из `master` (`f703574d`), выгружал базу с фикстурой
(`providers.dump: agent`, секции `tools.designer_agent` нет, `v8-runner --json-message dump
--force`). Он поднял агента так:

```text
1cv8 DESIGNER /IBConnectionString File=<ib> /AgentMode /AgentPort 51785 /AgentListenAddress 127.0.0.1 /AgentSSHHostKey <work>/agent/host-keys/host-key-<pid>-<uuid> /AgentBaseDir <work>/agent/base
```

| | 8.3.27.2074 | 8.5.4.1878 |
| --- | --- | --- |
| файл ключа | `0600`, `-----BEGIN OPENSSH PRIVATE KEY-----`, открытая часть `ssh-ed25519 … v8-runner managed agent` | то же |
| `ssh-keyscan -t ed25519 -p <порт> 127.0.0.1` | тот же открытый ключ; `SHA256:U+qOj2KS4u0OKTX1rjTPuvswoA4Tt541L9Em8QFX5Xw` и у `ssh-keygen -lf <файл>`, и у опубликованного | то же, `SHA256:Sg0tC/BHVX1iOkNnUJo18d5vZPJP6WURRqSwETFULjU` |
| порт | 51785 — из привязки к `127.0.0.1:0` | 52551 |
| исход | сессия закреплена на ключе, `ok: true`, 51 файл, 9 с; файл ключа удалён, процесса агента нет | то же, 11 с |

Баннер SSH на 8.3.27.2074 — `SSH-2.0-libssh_0.9.3`. Все порты замера (51477–62048) выбраны
привязкой к порту 0, то есть из эфемерного диапазона macOS 49152–65535, и приняты обеими
платформами. Ключ `ssh-keygen -t ed25519 -C "v8-runner managed agent"` с правами `0600`
агент 8.3.27 тоже принимает и публикует.

**Занятый `/AgentPort`.** Агент запускался той же командной строкой с `/Out <файл>` и без него
на порт, который уже держали:

| кто держит порт | 8.3.27.2074 | 8.5.4.1878 |
| --- | --- | --- |
| слушатель на `127.0.0.1:<порт>`, не SSH | выход **rc=0** через 2,6 с | **rc=0** через 22,3 с |
| слушатель на `0.0.0.0:<порт>` | rc=0, 2,7–3,1 с | rc=0, 22,4 с |
| другой агент 8.3/8.5 на своей копии базы | rc=0, 2,6 с; первый агент продолжает отвечать | rc=0, 22,3 с; то же |

stdout и stderr процесса пусты. Текст есть только в файле `/Out`: «Фатальная ошибка
SSH-сервера: Binding to 127.0.0.1:<порт>: Address already in use». До выхода агент держится
столько же, сколько обычно поднимается до слушателя: 8.3.27 — около 2 с, 8.5.4 — около 21 с.
Попутно: файла ключа нет — тоже rc=0 (2,6 с на 8.3.27, 11 с на 8.5.4), в `/Out` «Фатальная
ошибка SSH-сервера: Ошибка установки ключа хоста».

**Что видит раннер сейчас** (тот же `f703574d`, 8.3.27, объявленный `port`,
`startup_timeout_ms: 60000`):

| кто держит порт | ответ раннера |
| --- | --- |
| слушатель, не SSH | через **61 с** `environment_unavailable`: «managed agent did not accept a session within 60000 ms; last: agent at 127.0.0.1:<порт> is unreachable: Connection reset by peer (os error 54)», хотя агент вышел через 2,6 с |
| другой агент | через 0,37 с `environment_unavailable`: «… answered with host key SHA256:…, not with the key the runner handed to the agent it launched (SHA256:…): another SSH server holds the port …» |

**Вывод для потребителей.**

- Ключ раннера (OpenSSH, ED25519, комментарий `v8-runner managed agent`, `0600`) и
  свободный эфемерный порт обе платформы принимают; опубликованный отпечаток равен отпечатку
  файла.
- Отличимого кода выхода у занятого порта нет: rc=0, как у любого выхода агента. Отсутствие
  слушателя к сроку тоже не годится: порт слушает чужой процесс. Структурный сигнал один —
  **процесс агента завершился раньше первой аутентифицированной сессии**. Причину («порт
  занят», «нет ключа») называет только проза `/Out`, а раннер `/Out` агенту не передаёт.
- Раннер этот выход сейчас не замечает и ждёт весь `startup_timeout_ms` (по умолчанию
  120 с).
- Без лицензии (8.5.4.1878, тот же день) агент поднимается и слушает порт, но отвергает
  вход по SSH с пустой парой. Раннер отвечает «agent at … rejected the credentials of user
  ''», а пакетный Конфигуратор — «Не найдена лицензия…».

## Разрыв сессии с агентом во время долгой команды

Замер 08.10.2026. Задача: [#296](https://github.com/IngvarConsulting/v8-runner-rust/issues/296), п. 1;
методика — раздел «Отмена у чужого агента и шлюза SSH». Платформа 8.3.27.2074, macOS 27.0
(arm64). Путь `attached`: агента поднимал не раннер, а замер.

**Стенд.** Файловая база с фикстурой и 10 000 общими модулями по 63 КБ текста; выгрузка —
20 051 файл. Пакетный `/DumpConfigToFiles` — 95 с, агент — 81 с. Клиент SSH — свой на
`russh` 0.63, как у раннера: без псевдотерминала, пустая пара. Сессия: `options set
--show-prompt=no --output-format=json`, `common connect-ib`, затем `config
dump-config-to-files --dir=dump`. Разрыв — через 3 с после появления первых файлов. Каждую
секунду записывались число файлов, процессорное время `1cv8`, а раз в 11–13 с — новая
сессия с `common connect-ib`.

```text
1cv8 DESIGNER /IBConnectionString File=<W>/ibbig /AgentMode /AgentPort <свободный> /AgentListenAddress 127.0.0.1 /AgentSSHHostKey <W>/key /AgentBaseDir <W>/base
```

| вариант | после разрыва | новая сессия | итог |
| --- | --- | --- | --- |
| В, контроль: сессию не трогать | ответ `success` на 81,0 с | — | 20 051 файл |
| А: `common disconnect-ib`, сразу EOF, закрытие канала и `disconnect` соединения (на 4,1 с) | файлы растут до 20 051, процессорное время 6 → 86 с | до 76,8 с отказ запроса shell (`channel request` → failure); на 90,2 с shell есть, `connect-ib` — `success` | 20 051 файл, `diff -rq` с контролем пуст |
| Б: клиент убит, соединение закрыто без `disconnect-ib` (на 4,05 с) | так же | до 76,2 с отказ shell, на 90,0 с `success` | так же |
| А2: `common disconnect-ib` без закрытия сессии | ответ на выгрузку на 89,8 с, ответ `success` на `disconnect-ib` — на 91,6 с | — | так же |

**Как читать.** Разрыв сессии команду не снимает: выгрузка идёт до конца, а её результат
полон и побайтно равен контрольному и пакетному. `disconnect-ib` shell исполняет только после
текущей команды. Пока команда идёт, агент не даёт новой сессии даже shell: соединение и
вход проходят, а запрос shell отвергается. Следующая сессия раннера к этому агенту до конца
команды получит отказ на запросе shell, а не ответ «база занята».

`infobase-tools dump-ib` на этой базе выполняется меньше секунды (`.dt` 3,8 МБ, конфигурация
внутри сжата), долгой команды из неё не вышло — не мерено. Шлюз автономного сервера
(`gate`) и 8.5.4.1878 не мерены: шлюз в этот замер не входил, а 8.5.4 в тот же день не
нашла лицензии.

## Поколение после применения и после загрузки без применения

Замер 08.10.2026. Задача: [#210](https://github.com/IngvarConsulting/v8-runner-rust/issues/210)
(`apply`, `push --no-apply`). Платформа 8.3.27.2074, macOS; файловая база, каждое состояние —
на своей копии каталога базы. Фикстуры `tests/fixtures/designer/configuration` и
`tests/fixtures/designer/extension` (`Расширение1`); изменение основной — строка комментария в
конце `CommonModules/ОбщийМодуль1/Ext/Module.bsl`, расширения — `<Comment>` параметра сеанса
`Расш1_ПараметрСеанса1`. В каждой точке токен читали оба инструмента: Конфигуратор (Д) и `ibcmd` (И).

```text
ibcmd infobase --data <W>/ib_data --db-path <W>/ib create
1cv8 DESIGNER /DisableStartupDialogs /DisableStartupMessages /F <W>/ib /Out <f> -NoTruncate /GetConfigGenerationID [-Extension Расширение1]
ibcmd infobase --data <W>/ib_data --db-path <W>/ib config generation-id [--extension=Расширение1]
1cv8 DESIGNER … /LoadConfigFromFiles <W>/src1 [-files CommonModules/ОбщийМодуль1/Ext/Module.bsl]
1cv8 DESIGNER … /LoadConfigFromFiles <W>/ext1 -Extension Расширение1
1cv8 DESIGNER … /UpdateDBCfg [-Extension Расширение1]
1cv8 DESIGNER … /LoadConfigFromFiles <W>/src1 /UpdateDBCfg                    # одним запуском
1cv8 DESIGNER … /LoadConfigFromFiles <W>/ext1 -Extension Расширение1 /UpdateDBCfg
ibcmd infobase … config import [--extension=Расширение1] <W>/src1
ibcmd infobase … config apply [--extension=Расширение1] --force --dynamic=auto
ibcmd infobase … config reset
```

Все вызовы ниже — rc=0; чтение — около 2,3 с у Конфигуратора и 4,5 с у `ibcmd`, загрузка и
применение — 2,5–5,5 с. Токены сокращены до первых восьми знаков; у основной конфигурации все
они имеют вид «32 hex и `00000000`», у расширения — 40 hex без нулевого хвоста.

**Основная конфигурация.** Д и И отдают одно и то же значение во всех состояниях.

| состояние | Д = И |
| --- | --- |
| пустая база | сорок нулей |
| `/LoadConfigFromFiles` без применения | `459b5862` |
| затем `/UpdateDBCfg` (база S1) | `459b5862` — **не изменился** |
| S1 + полная загрузка изменённого модуля без применения | `b86e3d32` |
| то же, загрузка тех же файлов ещё раз | `dfba80e8` — снова новый |
| S1 + частичная загрузка (`-files`) без применения | `94c95888` |
| S1 + `ibcmd config import` без применения | `17d6d9a3` |
| загрузка, затем `/UpdateDBCfg` | прежний токен загрузки (`b86e3d32`, `94c95888`, `17d6d9a3`) |
| загрузка, затем `ibcmd config apply --force --dynamic=auto` | прежний токен загрузки (`b86e3d32`, `17d6d9a3`) |
| S1 + `/LoadConfigFromFiles … /UpdateDBCfg` одним запуском | `a0e1ae6e` — новый, того же вида |
| S1 + загрузка того же дерева, что уже применено, затем `/UpdateDBCfg` | `b2d4f37e` после загрузки, после применения тот же |
| загрузка без применения, затем `ibcmd config reset` | `459b5862` — значение S1 вернулось |
| повторное применение без изменений (`/UpdateDBCfg` или `config apply`, на S1 и после применения) | без изменений |

Пустое `/UpdateDBCfg` пишет пустой `/Out` с rc=0, пустое `config apply` — обычные строки
`[INFO] …`. Загрузки и применения основной конфигурации токены расширения не меняют.

**Расширение** `Расширение1`. Здесь **Д и И читают разное**.

| состояние | Д | И |
| --- | --- | --- |
| загружено в первый раз, не применено | сорок нулей | `c4cac2b6` |
| применено (S1) | `c4cac2b6` | `c4cac2b6` |
| S1 + `/LoadConfigFromFiles -Extension` или `config import --extension`, без применения | `c4cac2b6` | `3920612b` |
| то же, загружено дважды | `c4cac2b6` | `3920612b` |
| затем загружено прежнее дерево `ext0`, без применения | `c4cac2b6` | `c4cac2b6` |
| загрузка, затем `/UpdateDBCfg -Extension` или `config apply --extension` | `3920612b` | `3920612b` |
| S1 + `/LoadConfigFromFiles ext1 -Extension Расширение1 /UpdateDBCfg` одним запуском | `3920612b` | `3920612b` |
| повторное применение без изменений (обоими инструментами) | без изменений | без изменений |

`-Extension` дважды в одном запуске (`/LoadConfigFromFiles … -Extension Р /UpdateDBCfg -Extension Р`) —
rc=1, `/Out` «Ошибка в параметрах командной строки.». `/UpdateDBCfg -Extension Р /LoadConfigFromFiles
<ext1>` — rc=1, «Загрузка не должна менять принадлежность основного объекта конфигурации»: ключ
относится к одной команде запуска, а не к обеим. Токены расширения у обоих отказов не изменились.

**8.5.4.1878 не замерена.** Конфигуратор на каждый вызов отвечал rc=1 за ~22 с, `/Out`:
«Не найдена лицензия. Не обнаружен ключ защиты программы или полученная программная лицензия!».
`ibcmd` не завершался: `infobase create` и первые `config generation-id` печатали результат
(`… успешно завершено`, `2af84151e959af78eab1cb38d137eedf33543af5`) и не выходили 300 с, позже —
ни строки за 60 с; повторы раз в ~2,5 мин с 13:05 до 13:42 дали то же. `sample` показывал главный поток в `uv_run` из
`InfoBaseManagementOfflineSession::run`. В то же время так же висел `ibcmd` 8.5 другого замера.
Процессы сняты по тайм-ауту. Вопрос о 8.5 остаётся открытым до рабочей лицензии и `ibcmd` 8.5 на стенде.

**Вывод для потребителей.** У основной конфигурации токен — поколение **основной** конфигурации:
он новый после каждой записи, даже того же содержимого, а применение (`/UpdateDBCfg`, `config apply --dynamic=auto`,
в отдельном запуске или в одном с загрузкой) его не меняет; `config reset`
возвращает токен конфигурации базы данных. Поэтому `apply` (#210) по токену не отличить ни от
пустого действия, ни от «ничего не делали»: «применено ли» отвечает признак непринятого
(раздел «Признаки „есть непринятое“…»), а не поколение; `push --no-apply` меняет токен так же, как
`push` с применением. Пустое применение токен не меняет. У расширения токен — по содержимому
(то же дерево — то же значение, возврат дерева — прежнее значение), и инструменты читают разные
стороны: `ibcmd --extension` — загруженное расширение, `/GetConfigGenerationID -Extension` —
применённое (до первого применения — сорок нулей). Совпадают они только у применённого
расширения; это и есть расхождение токенов расширения из раздела «Идентификатор поколения».
Сравнивать токены расширения можно лишь от одного инструмента, а у Конфигуратора загрузка
без применения токен расширения не меняет.
