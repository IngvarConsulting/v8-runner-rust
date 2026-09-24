# Конфигурационный контракт

Этот документ описывает поддержанный `v8project.yaml`: literal YAML keys, допустимые значения и
validation rules.

Каталог команд находится в [CAPABILITIES.md](CAPABILITIES.md), а runtime semantics и operational
nuances вынесены в [DEEP_DIVE.md](DEEP_DIVE.md).

## Навигация

- [Как получить стартовый конфиг](#как-получить-стартовый-конфиг)
- [YAML Schema и VS Code](#yaml-schema-и-vs-code)
- [Именование ключей](#именование-ключей)
- [Канонический пример](#канонический-пример)
- [Локальный overlay](#локальный-overlay)
- [Обязательный контракт](#обязательный-контракт)
- [Опциональные секции](#опциональные-секции)
- [`tools.platform`](#toolsplatform)
- [`tools.enterprise`](#toolsenterprise)
- [`tools.edt_cli`](#toolsedt_cli)
- [`tools.designer_agent`](#toolsdesigner_agent)
- [Неподдержанные ключи](#неподдержанные-ключи)

## Как получить стартовый конфиг

Базовый файл можно сгенерировать командой:

```bash
v8-runner init
```

Что делает `init`:

- создаёт `v8project.yaml` в текущем каталоге или по `--output <FILE>`;
- добавляет modeline `yaml-language-server` со ссылкой на опубликованный schema artifact в
  ветке `master`;
- создаёт рядом `v8project.local.yaml` с modeline на
  `https://raw.githubusercontent.com/IngvarConsulting/v8-runner-rust/master/docs/schemas/v8project.local.schema.json`
  и объявляет в нём базу `origin`: адрес из `--connection`, по умолчанию `File=build/ib`.
  Существующий местный слой сохраняется: `origin` дописывается, если не объявлен; секция
  без адреса (прежний `infobase:` только с учётными данными) получает адрес и переезжает в
  `infobases.origin`; объявленный адрес, отличный от `--connection`, — отказ. Файл с картой
  `infobases` или потоковой записью при дописывании перезаписывается целиком, комментарии
  в нём теряются;
- добавляет `v8project.local.yaml` в `.gitignore`, если подходящий pattern еще не указан;
- заполняет `source-set` по найденным исходникам;
- не перезаписывает существующий файл без `--force`;
- не пишет synthetic `CONFIGURATION`: если конфигурационный `source-set` не найден,
  завершается validation error;

Автообнаружение опирается на содержимое marker files, а не на имена каталогов:

- Designer ordinary sources находятся по `Configuration.xml`, а их тип определяется по XML;
- Designer external aggregate root создаётся как один `source-set` только при однородных
  top-level XML descriptors;
- EDT ordinary projects находятся по `.project`, `DT-INF/PROJECT.PMF` и native markers под `src`;
- EDT external root создаётся только если direct child projects однородно классифицируются как
  один external kind.

После загрузки конфига относительные пути резолвятся относительно каталога, где лежит
`v8project.yaml`.

Если рядом с основным конфигом есть `v8project.local.yaml`, он применяется автоматически после
`v8project.yaml` и до CLI overrides. Локальный файл объявляет базы проекта (карта
[`infobases`](#infobases)) и держит machine-local пути, credentials и runtime настройки; его
следует держать вне Git.

## YAML Schema и VS Code

Начиная с `a1db1f8f422ca1bf71a04c1b4793d27eb8c6d0b4`, в репозитории есть
schema artifacts для редактирования `v8project.yaml` и `v8project.local.yaml` в IDE.

В репозитории публикуются две JSON Schema:

- `docs/schemas/v8project.schema.json` для основного `v8project.yaml`;
- `docs/schemas/v8project.local.schema.json` для локального overlay `v8project.local.yaml`.

`v8-runner init` пишет в начало `v8project.yaml` modeline:

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/IngvarConsulting/v8-runner-rust/master/docs/schemas/v8project.schema.json
```

В VS Code установите расширение `redhat.vscode-yaml`. Оно использует эту строку
автоматически; отдельная настройка workspace для основного файла не нужна.

Для `v8project.local.yaml` `init` пишет отдельную modeline:

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/IngvarConsulting/v8-runner-rust/master/docs/schemas/v8project.local.schema.json
```

Если local overlay создаётся вручную, добавьте это в `.vscode/settings.json` проекта или в user
settings:

```json
{
  "yaml.schemas": {
    "https://raw.githubusercontent.com/IngvarConsulting/v8-runner-rust/master/docs/schemas/v8project.local.schema.json": "v8project.local.yaml"
  }
}
```

Schema URL всегда указывает на `master`, чтобы IDE подхватывала актуальный опубликованный schema
artifact без привязки к release tag.

## Именование ключей

`v8project.yaml` использует не один стиль на весь документ. Это текущий loader contract, и docs
ниже повторяют именно literal YAML keys.

- top-level app keys: `workPath`, `format`, `providers`, `source-set`, `push`, `tools`,
  `mcp`, `tests`; базы объявляет местный слой ключом `infobases`;
- `push` использует `partialLoadThreshold`;
- `mcp.*` и `tests.*` используют `snake_case`;
- canonical key для EDT tool section: `tools.edt_cli`;
- у `tools.edt_cli` literal child keys смешанные:
  - `interactive-mode`
  - `auto-start`
  - `startup_timeout_ms`
  - `command_timeout_ms`
- canonical key для агента Конфигуратора: `tools.designer_agent`; child keys —
  `attach`, `base-dir`, `port`, `host-key` и `startup_timeout_ms`.

Ключи названы именами команд. Прежние написания приняты ещё один цикл выпуска и помечены
`deprecated` в опубликованной схеме: `providers.init` → `providers.infobase.create`,
`providers.build` → `providers.push`, `providers.load` → `providers.upload`,
`providers.dump` → `providers.pull`, `providers.infobase.configuration.export` →
`providers.download`, секция `build:` → `push:`. Прежнее имя секции загрузчик сворачивает в
новое с предупреждением; оба ключа в одном файле — отказ, потому что в карте они схлопнулись
бы молча. Квитанция ответа называет ключ новым именем, как бы его ни написали в файле.

Ниже фиксируются только поддержанные canonical keys.

## Канонический пример

Проектный файл базы не называет: к какой базе подключён каталог, знает эта машина, и базы
объявляет [локальный overlay](#локальный-overlay).

```yaml
workPath: build
format: EDT

source-set:
  - name: main
    type: CONFIGURATION
    path: main
  - name: ext
    type: EXTENSION
    path: ext

push:
  partialLoadThreshold: 20

tools:
  client_mcp:
    port: 9874
    extension:
      name: client_mcp
      source:
        path: /path/to/onec-client-mcp/exts/client-mcp
        format: EDT
  va:
    epf_path: /path/to/vanessa.epf
  platform:
    path: /opt/1cv8/x86_64
    strict: true
    version: 8.3.27.1859
  enterprise:
    additional-launch-keys:
      - /TESTMANAGER
  edt_cli:
    path: 2025.2.3
    version: 2025.2.3
    interactive-mode: false
    auto-start: false
    startup_timeout_ms: 300000
    command_timeout_ms: 300000
  designer_agent:
    port: 1543               # управляемый агент; либо attach: host:port ([v6]:port)
    startup_timeout_ms: 120000

mcp:
  http:
    bind_address: 127.0.0.1:3000
    path: /mcp
    stateful_sessions: true
    max_sessions: 64
    idle_ttl_secs: 900
  execution:
    max_concurrent_calls: 1
    shutdown_grace_period_secs: 30
    admission_timeout_ms: 300000

tests:
  execution_timeout_seconds: 300
  yaxunit:
    timeouts:
      total_ms: 300000
  va:
    params_path: /path/to/va-params.json
    profile: smoke
    fail_fast: false
    timeouts:
      total_ms: 300000
    profiles:
      smoke:
        feature_path: /path/to/features
```

## Локальный overlay

`v8project.local.yaml` расположен рядом с выбранным primary config и применяется автоматически.
`init` создаёт пустой local overlay как валидный YAML mapping (`{}`), добавляет schema
modeline и сохраняет существующие значения, если файл уже был создан вручную. Файл не является
самостоятельным config entrypoint: передавать его через `--config` нельзя.

Precedence:

1. `v8project.yaml`;
2. `v8project.local.yaml`, если существует;
3. CLI overrides, например `--workdir`.

Merge rules:

- object/map значения merge-ятся рекурсивно;
- scalar значения из local overlay заменяют project значения;
- list значения заменяются целиком;
- `null` работает как обычное YAML-значение и допустим только для optional typed fields;
- относительные пути local overlay резолвятся относительно каталога primary config.

Local overlay может задавать machine-local секции:

- `workPath`;
- `infobases.<имя>.*` — секции баз целиком: `connection`, `user`/`password`, `dbms`,
  `cluster`, `web`, `standalone`; `origin` — умолчание;
- `infobase` — синоним `infobases.origin` на один цикл выпуска;
- `tools.*`;
- `tests.*`;
- `mcp.*`.

Другие top-level ключи в local overlay отклоняются.

Local overlay не может менять project identity:

- `source-set`;
- `format`.

Пример:

```yaml
workPath: build-local

infobases:
  origin:
    connection: "File=local/ib"
    user: Admin
    password: secret
  test:
    connection: "Srvr=srv;Ref=erp_test"

tools:
  platform:
    path: /opt/1cv8/x86_64
  va:
    epf_path: /home/user/tools/vanessa.epf

tests:
  va:
    params_path: /home/user/project/.local/va-params.json
```

## Обязательный контракт

### `workPath`

- Тип: путь
- Обязателен: да

Для обычных project-команд список должен содержать поддерживаемый source-set. Исключение —
`download` и `infobase dump`: они используют только ИБ и принимают
отсутствующий `source-set` как пустой список (явный `source-set: []` равнозначен). Это
command-specific validation, а не ослабление `push`, source `pull`, `convert`, `make` или
остальных project workflows.

Корень runtime state:

- `workPath/hash-storages`
- `workPath/logs`
- `workPath/temp`
- `workPath/edt-workspace`
- `workPath/designer`

Если каталога нет, он создаётся автоматически при захвате workspace lock. Pure provider
selection для infobase export не создаёт `workPath` и runtime-файлы.

### `execution_timeout` — изъят

Ключ больше не поддерживается: у команды нет срока, она идёт до терминального исхода
(`DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE`). Конфигурация с этим ключом отклоняется
именным отказом. Предел задаётся шагу, которому он нужен:
`tools.edt_cli.command_timeout_ms`, `tests.execution_timeout_seconds`, блоки
`tests.*.timeouts`, `tools.client_mcp.wait_ready_timeout_ms`. Ожидание свободного слота
у MCP-вызова ограничивает `mcp.execution.admission_timeout_ms`.

### `format`

- Тип: enum
- Значения: `DESIGNER`, `EDT`
- По умолчанию: `DESIGNER`

### `providers`

- Тип: объект `операция → исполнитель`
- Обязателен: нет

Исполнителя каждой операции раннер выбирает сам по матрице возможностей — паре
«операция и вид информационной базы». Ключ нужен только для того, чтобы назначить
исполнителя вручную: поставить эксперимент или обойти сломанное умолчание.

```yaml
providers:
  push: ibcmd
  download: ibcmd
```

Правила:

- значение — одно имя из закрытого набора `designer`, `agent`, `ibcmd`, `ibcmd-rs`,
  `webinst`; назначенный исполнитель обязан реализовывать операцию на этой базе;
- ключ принимается только для операции, у которой на этой базе есть выбор; для
  операции с одним исполнителем это ошибка конфигурации, а не подтверждение очевидного;
- переопределение строгое: если названный исполнитель не готов, команда отказывает с
  причиной и на умолчание не откатывается;
- допустимые ключи: `infobase.create`, `push`, `upload`, `pull`, `extensions`,
  `download`, `infobase.dump`, `infobase.restore`, `syntax`, `make`;
- ключ разрешён и в `v8project.local.yaml` — для машинно-локального эксперимента; в
  квитанции ответа видно, из какого файла он пришёл.

Умолчания по операциям: `infobase.create`, `push`, `pull` — Конфигуратор, затем `ibcmd`;
`download` — Конфигуратор, затем `ibcmd`; `infobase dump` и
`infobase restore` — Конфигуратор (`ibcmd` для DT остаётся экспериментальным и
назначается только явно); `upload`, `syntax`, `make` — только Конфигуратор;
`extensions` — только `ibcmd`.

Ключ `builder` снят: конфиг с ним не проходит валидацию, а ошибка называет замену.

### `infobases`

Базы проекта объявляются в `v8project.local.yaml` картой по именам, как `remote` у
репозитория; проектный файл базы не называет. Умолчание — `origin`: команды идут в неё без
ключа. `--infobase <имя>` выбирает другую объявленную базу; `--infobase <строка соединения>`
— базу ad hoc: без имени, без учётных данных (`Usr=`/`Pwd=` или `/N`/`/P` в строке — отказ),
и это не автономный сервер — его объявляют секцией `standalone`. Без `origin` и без ключа
команда отказывает до запуска платформы и называет шаг. Ключ действует и на `mcp serve`.

Имя базы — идентификатор `[A-Za-z0-9][A-Za-z0-9_-]{0,63}`: оно же становится каталогом под
`workPath`, поэтому другого имени схема не принимает.

```yaml
# v8project.local.yaml
infobases:
  origin:
    connection: "File=build/ib"
  test:
    connection: "Srvr=srv;Ref=erp_test"
    user: tester
    password: secret
```

**Синоним на один цикл выпуска.** Прежний ключ `infobase:` читается как `infobases.origin`
— в проектном файле и в местном слое, в каждом с предупреждением (в тексте — узел
`▲ config:` перед лентой команды, в JSON — `warnings` конверта); схема помечает его как
`deprecated`. Карта `infobases` в проектном файле не принимается. Оба ключа в одном файле
— отказ. Проектный `infobase:` и местный `infobases.origin` — одна база, слитая по полям:
так `connection` из проектного файла и `user`/`password` из местного продолжают работать
вместе до переезда секции. В следующем цикле выпуска синоним снимается.

Ниже `infobase.<поле>` — поле секции базы, объявленной под любым именем.

#### `infobase.connection`

- Тип: строка
- Обязателен: да, если цель не автономный сервер

Строка подключения к ИБ. Поддерживается непустой `File=...`, server connection
`Srvr=...;Ref=...` (завершающая `;` и кавычки допустимы) или аргументная форма
`/S server\ref`. Для file-based ИБ относительный `File=...` резолвится относительно каталога
конфига. Объявленную серверную строку раннер отдаёт платформе в её собственной форме:
`Srvr=srv:1541;Ref=demo` уходит как `/S srv:1541\demo`, а `user`/`password` — отдельными
`/N`/`/P`; рядом с `/IBConnectionString` платформа 8.3.27.1936 на Windows реквизиты не
принимала — «Пользователь ИБ не идентифицирован» (#55).

Форму `/S` получает только строка из двух частей `Srvr` и `Ref` с одной машиной в адресе.
Строку с `Usr=`/`Pwd=` раннер не разбирает и отдаёт целиком: свои реквизиты она несёт сама,
и `user`/`password` рядом с ней не нужны. Строка с иными дополнительными частями
(`Locale=`, список резервных серверов `Srvr='srv1,srv2:1641'`) тоже уходит целиком, потому
что ключ `/S` их не несёт, — а вместе с ней `user`/`password` на 8.3.27.1936 могут не
сработать по той же причине, что в #55. Держите реквизиты в `user`/`password`, а адрес —
из двух частей; если дополнительная часть нужна, назовите базу сырой формой
`/S host\ref`. Вид цели отвечает на три вопроса по порядку: есть секция
[`infobase.standalone`](#infobasestandalone) — автономный сервер; иначе в строке есть `File=` —
файловая база; иначе — кластер, и строка обязана быть серверного вида, произвольная непустая
строка — отказ валидации. Рядом с секцией `standalone` строка — адрес прямого шлюза
`Srvr=<host[:port]>;Ref=<name>` или та же строка в сырой форме `/S host\name` (пока принимается,
но не используется: команды идут через `standalone.gate`, Конфигуратор по прямому шлюзу — #205)
либо пусто; `File=` и любая другая строка рядом с ней — отказ.

#### `infobase.standalone`

- Тип: объект
- Обязателен: нет

Автономный сервер (`ibsrv`) как цель. Раннер его **не запускает** и не владеет его
флагами: как поднят сервер — дело пользователя. Раннер подключается к SSH-шлюзу
сервера (`ibsrv --enable-ssh-gate`, порт 1543 по умолчанию) и выполняет через него те же
команды, что через агент Конфигуратора; `infobase.user`/`infobase.password` — пользователь
базы, как для шлюза требует платформа. В `gate` порт указывается всегда (`host:port`, IPv6 —
в скобках): к шлюзу идёт собственный SSH-клиент раннера, умолчания платформы у него нет.

```yaml
infobases:
  origin:
    user: Admin
    password: secret
    standalone:
      gate: srv.example:1543      # host:port SSH-шлюза; IPv6 — в скобках: '[::1]:1543'
      host-fingerprint: 'SHA256:…'  # чей ключ считать своим; не объявлен — принимается любой
      exchange: sftp              # файлы — по SFTP того же шлюза
    web:
      url: http://srv.example:8314/demo   # адрес для launch web
```

`host-fingerprint` — отпечаток ключа хоста шлюза в виде `SHA256:<base64>`, то, что
печатают `ssh-keyscan` и `ssh-keygen -lf`. Объявлен — принимается только этот ключ, иначе
типизированный отказ; не объявлен — ключ принимается, а отпечаток называется в
предупреждении, чтобы его можно было закрепить одной строкой. Для сервера на другой
машине это единственное, что отличает его от того, кто занял его адрес: учётные данные
пользователя базы уходят сразу после рукопожатия.

Файловая система сервера раннеру не принадлежит, даже когда они на одной машине:
пути в командах шлюза разрешаются на стороне сервера относительно каталога пользователя
(`<users-data>/<user>`), а файлы туда и обратно идут только **объявленным каналом**:

- `exchange: sftp` — подсистема SFTP того же SSH-соединения; корень — каталог пользователя
  шлюза. Так работают с сервером на другой машине: исходники и списки уходят на сервер,
  результаты возвращаются, следов прогона на сервере не остаётся. Инкрементальная
  выгрузка не возит цель целиком: на сервер уходит только опись `ConfigDumpInfo.xml`,
  обратно — изменённые файлы поверх локальной цели; частичная сборка возит только
  `Configuration.xml`, `ConfigDumpInfo.xml`, изменённые файлы и список. Шлюз `ibsrv` 8.3.27 отдаёт по SFTP
  чтение и каталоги, а **запись не принимает** ни с какими флагами открытия (замер
  15.09.2026): через него по SFTP работают `pull --mode full`, `make` и экспорт `.cf`,
  а `push` и инкрементальная выгрузка отказывают типизированно, с путём и кодом шлюза;
  на стороне сервера при этом ничего не остаётся. Раннер пробует сочетания флагов
  открытия по убыванию строгости, так что точка входа, принимающая запись (агент
  Конфигуратора по документации), получит и загрузку.
- `exchange: { dir: /srv/ib/users-data/Admin }` — каталог пользователя шлюза на машине
  раннера или его монтирование; файлы выставляются ссылками и переносятся без сети.

Без объявленного канала любая команда отказывает валидацией. `workPath` при этом
остаётся на стороне раннера и не может лежать внутри `exchange.dir`.
`tools.designer_agent` к автономному серверу не относится и с этой секцией отвергается;
`infobase.dbms` и `infobase.cluster` — тоже: `ibsrv` — один процесс вместо кластера, и
`ras` им не управляет.

У автономного сервера один исполнитель — `agent` через шлюз — для `push`, `pull`,
`make`, `extensions` и `download`. `infobase dump` и `infobase restore`
через шлюз не выполняются намеренно: `infobase-tools dump-ib` роняет `ibsrv` 8.3.27
(замер 15.09.2026), а `restore-ib` по документации завершает сеанс сервера — снимок
автономного сервера снимают его собственными средствами. `upload` (у шлюза нет
`compare-cfg`), `check`, `infobase create`, `publish` и `test` отказывают типизированно. Клиент
открывается по `infobase.web.url`: либо `launch web` в браузере, либо `launch thin` —
тонкий клиент идёт по тому же адресу ws-соединением, без всякого ключа, потому что
объявленная строка прямого шлюза раннером пока не используется (#205), а `--via connection`
отвергается. Конфигуратор,
толстый клиент и обычное приложение против
неё по-прежнему отказывают. Реквизиты базы тонкому клиенту не передаются: у автономной
цели `infobase.user` и `infobase.password` — учётные данные шлюза, а не базы. После `update-db-cfg` сервер 15–20 с «обновляется» и отвергает
SSH-логин — раннер ждёт шлюз до 30 с.

#### `infobase.user` / `infobase.password`

- Тип: строка
- Обязательны: нет

Credentials самой информационной базы — первый из трёх уровней учётных данных; два
других, администратор кластера и администратор центрального сервера, лежат в
[`infobase.cluster`](#infobasecluster). Каждый уровень запрашивает только та операция,
которой он нужен.

#### `infobase.web`

- Тип: объект
- Обязателен: нет

У базы два адреса. По `infobase.connection` раннер её **администрирует**; по
`infobase.web.url` её **открывают** клиентом или браузером. Строка `ws=…` в
`infobase.connection` не принимается: она называет второй адрес, а не первый, и чем
администрировать базу, из неё не следует.

```yaml
infobases:
  origin:
    connection: "Srvr=srv:1541;Ref=demo"
    web:
      server: apache24            # iis | apache2 | apache22 | apache24
      wsdir: demo                 # виртуальный каталог
      dir: /var/www/demo          # физический каталог, должен существовать
      conf: /etc/httpd/httpd.conf # обязателен для apache2 и apache22
      os-auth: false              # только для iis
      url: http://localhost/demo  # адрес для launch web
```

`server`, `wsdir` и `dir` нужны команде `publish`; `url` — команде `launch web`. У
файловой и кластерной базы адрес появляется после публикации, у автономного сервера
известен сразу. Секция лежит в местном слое вместе с остальной секцией базы.

#### `infobase.dbms`

- Тип: объект
- Обязателен: нет

Нужна там, где раннер идёт в СУБД напрямую: создать серверную информационную базу
(`infobase create` с `providers.infobase.create: ibcmd`) и любая операция `ibcmd` с серверной
базой — `ibcmd` работает с её данными сам; так, `upload` расширения проверяет его наличие
через `ibcmd`. Где `ibcmd` не вызывается, секция не требуется, и валидацией она не
запрашивается.

Поддержанные поля:

- `kind`
- `server`
- `name`
- `user`
- `password`

#### `infobase.cluster`

- Тип: объект
- Обязателен: нет

Кластер вокруг серверной базы: адрес сервера администрирования и два уровня
администраторов над пользователем базы. Все поля необязательны.

```yaml
infobases:
  origin:
    connection: "Srvr=srv:1541;Ref=demo"
    user: Admin                 # пользователь базы — первый уровень
    password: secret
    cluster:
      ras: srv:1545             # сервер администрирования, host[:port]
      user: cluster-admin       # администратор кластера — второй уровень
      password: cluster-secret
      agent:
        address: srv:1540       # агент центрального сервера, host[:port]; по умолчанию —
                                # хост из Srvr= и порт платформы
        user: agent-admin       # администратор центрального сервера — третий уровень
        password: agent-secret
```

Поддержанные поля:

- `ras` — адрес сервера администрирования (`host` или `host:port`, IPv6 в скобках:
  `[::1]:1545`). Раннер по этому адресу сам не звонит: он уходит `rac` как есть, и порт
  по умолчанию (1545) остаётся за платформой; поэтому, в отличие от `standalone.gate`, порт
  не обязателен.
- `user`, `password` — администратор кластера.
- `agent.address` — адрес агента центрального сервера в той же форме; нужен, когда раннер
  поднимает `ras` сам, а агент отвечает не по хосту из `Srvr=` с портом платформы (1540).
- `agent.user`, `agent.password` — администратор центрального сервера; ни одна операция
  раннера сама его не требует.

Три уровня лежат в местном слое порознь, и каждый запрашивает только та операция,
которой он нужен: `sessions list` и `sessions terminate` — кластер; `sessions deny` и
`sessions allow` — кластер и база; `infobase create` в кластере с заполненным списком
администраторов — кластер через `SUsr`/`SPwd` строки создания. Отказ называет, какого
уровня не хватает. Сегодня секцию проверяет только валидация — формы адресов и места:
рядом с `File=` и рядом с `standalone` она отвергается, потому что у файловой базы и у
автономного сервера кластера нет. Команды, которые её читают, приходят позже: `sessions`
(#212), `ras`, поднятый раннером без `cluster.ras` (#213), `infobase create` в кластере
(#204). Пустые `cluster: {}` и `agent: {}` принимаются молча. Синоним `infobase:` в
проектном файле на этот цикл принимает секцию вместе с остальными полями базы — как и
`password`; место ей в местном слое.

### `source-set`

- Тип: список
- Обязателен: да

Каждый элемент содержит:

- `name`
- `type`
- `path`

`path` задаётся относительно каталога primary `v8project.yaml`, если он не абсолютный.

`type` поддерживает только:

- `CONFIGURATION`
- `EXTENSION`
- `EXTERNAL_DATA_PROCESSORS`
- `EXTERNAL_REPORTS`

Validation rules:

- `name` должен быть уникальным и безопасным path segment;
- `EXTENSION` требует хотя бы один `CONFIGURATION`, но external-only config допустим;
- для `format=DESIGNER` ordinary source-set должен указывать на корректный Designer root;
- для `format=DESIGNER` external source-set должен быть aggregate root с top-level XML
  descriptors matching declared `type`;
- для `format=EDT` ordinary `CONFIGURATION`/`EXTENSION` path должен быть valid EDT project root:
  каталог с `.project`, правильным nature, `DT-INF/PROJECT.PMF` и project-local native markers;
- для `format=EDT` external path должен быть каталогом direct child projects, и все найденные
  child projects должны совпадать с declared external `type`.

## Опциональные секции

### `push`

#### `push.partialLoadThreshold`

- Тип: integer
- По умолчанию: `20`
- Минимум: `1`

Порог между partial и full load.

CLI selector `v8-runner push --source-set <name>` использует `source-set[].name` как stable
runtime identity и не добавляет отдельное поле конфигурации. Если selector не задан, `push`
обрабатывает все `source-set`.

### `tests`

#### `tests.execution_timeout_seconds`

- Тип: integer
- По умолчанию: `300`
- Диапазон: `1..=86400`

#### `tests.yaxunit.timeouts.total_ms`

- Тип: integer

#### `tests.va`

Поддержанные поля:

- `params_path`
- `profile`
- `fail_fast`
- `timeouts.total_ms`
- `profiles.<name>.feature_path`
- `profiles.<name>.features_to_run`
- `profiles.<name>.filter_tags`
- `profiles.<name>.ignore_tags`
- `profiles.<name>.scenario_filter`

`v8-runner test va --feature`, `--filter-tag`, `--ignore-tag` и `--scenario-filter`
переопределяют соответствующие списки выбранного профиля только для текущего CLI-запуска.
Для функциональных `.feature`-сценариев и приемки агенты должны использовать `test va` или MCP
`run_all_tests` с `runner=vanessa`; дефолтный MCP `run_all_tests` без `runner=vanessa` запускает
YaXUnit.
По умолчанию `fail_fast: false`.
Для `СписокТеговОтбор` и `СписокТеговИсключение` в runtime `VAParams` runner удаляет один
ведущий `@`, если он указан в `profiles.<name>.filter_tags`, `profiles.<name>.ignore_tags`,
`--filter-tag` или `--ignore-tag`.

При генерации runtime `VAParams` runner добавляет `WorkspaceRoot` со значением каталога primary
`v8project.yaml`, если это поле отсутствует или равно `null` в `tests.va.params_path`.

Для Vanessa Automation обязательны:

- `tools.va.epf_path`
- `tests.va.params_path`
- `tests.va.profile`
- `tests.va.profiles.<name>.feature_path`

Поля `startup_ms` и `run_ms` внутри `tests.*.timeouts` зарезервированы и сейчас не влияют на
запуск.

### `mcp.http`

Поддержанные поля:

- `bind_address`, по умолчанию `127.0.0.1:3000`
- `path`, по умолчанию `/mcp`
- `stateful_sessions`, по умолчанию `true`
- `max_sessions`, по умолчанию `64`
- `idle_ttl_secs`, по умолчанию `900`
- `allowed_hosts`, по умолчанию пусто

Слушатель отвечает, только если заголовок `Host` называет петлю (`127.0.0.0/8`,
`::1`, `localhost`) или одно из имён в `allowed_hosts`; иначе — `403`. Заголовок
`Origin`, если он есть, проверяется по тому же списку. Порт при сверке не
учитывается. Запись в `allowed_hosts` — имя или числовой адрес, необязательный
порт игнорируется; ни масок, ни диапазонов.

Проверка закрывает одно: браузер на той же машине переразрешает своё имя в
`127.0.0.1` и обращается к слушателю как к своему. Подделать `Host` страница не
может, поэтому сверка имени её и отсекает.

Чего проверка не делает: она не закрывает слушатель от клиентов вне браузера.
`curl -H 'Host: 127.0.0.1:3000' http://10.0.0.5:3000/mcp` подставит любой
заголовок, а у самого слушателя аутентификации нет. Поэтому `bind_address` вне
петли открыт всем, кто до него дотянется, и единственная защита там — периметр.
Назвать чужое имя в `allowed_hosts` значит взять его на себя; при непетлевом
`bind_address` с пустым списком запуск говорит об этом предупреждением.

### `mcp.execution`

Поддержанные поля:

- `max_concurrent_calls`, по умолчанию `1`. Вызовы над одним `workPath` параллельно не идут
  и при большем значении: каталог держит замок, и второй вызов отказывает сразу.
- `shutdown_grace_period_secs`, по умолчанию `30`
- `admission_timeout_ms`, по умолчанию `300000`, диапазон `1..=86400000`. Ограничивает
  только ожидание свободного слота: у клиента протокола нет Ctrl+C, и занятый слот иначе
  держал бы очередь молча. Допущенный вызов идёт до терминального исхода, сроку не
  подчиняясь (`DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE`).

### `tools.client_mcp`

Поддержанные поля:

- `port`, опциональный порт клиентского MCP-сервера onec-client-mcp-devkit.
- `wait_ready_timeout_ms`, опциональный timeout для `launch mcp --wait-ready` и MCP
  `launch_app.waitReady` в миллисекундах; если не задан, ожидание длится пять минут.
  Других границ у него нет: срока у команды нет, и уменьшать это значение под общий
  бюджет больше не требуется.
- `extension`, опциональное tool extension для клиентского MCP-сервера.

`launch mcp` передаёт это значение как `mcpPort` внутри payload аргумента `/C runMcp...`
если CLI не указал `--mcp-port`.
`launch mcp --wait-ready` и MCP `launch_app` с `waitReady=true` используют этот порт для
проверки `http://127.0.0.1:<port>/mcp`, если порт не передан явно.
Ожидание готовности ограничивается `tools.client_mcp.wait_ready_timeout_ms`; без этой настройки
оно длится пять минут и ничем сверху не обрезается.
Для Vanessa Automation MCP используйте `launch mcp va --wait-ready` или MCP `launch_app` с
`utilityType=mcp`, `mcpScenario=va` и `waitReady=true`; bare `launch mcp` проверяет только client
MCP endpoint и не гарантирует наличие Vanessa tools.

`extension` поддерживает:

- `name`, обязательное безопасное имя расширения в ИБ;
- ровно один источник:
  - `source.path` и опциональный `source.format` (`DESIGNER` или `EDT`, по умолчанию global
    `format`);
  - `artifact.path` на существующий `.cfe` файл.

`tools.client_mcp.extension` не добавляется в `source-set` и не выбирается через `--source-set`.
`infobase create` импортирует EDT `source` в workspace, `push` подготавливает расширение
после project source-set, а `launch mcp` и `launch mcp va` расширение не устанавливают и
не обновляют.
Для `source` `push` хранит отдельный snapshot под `workPath/hash-storages`: повторный запуск с
неизменёнными исходниками пропускает export/load, а `push --full` принудительно
обновляет расширение.

`v8-runner tools download client-mcp` может заполнить этот блок в `v8project.local.yaml`:
с `--sources` он указывает `source.path` на
`build/tools/onec-client-mcp-devkit/exts/client-mcp` и `source.format: EDT`, без
`--sources` указывает `artifact.path` на скачанный `client_mcp.cfe`. Artifact-режим
доступен, только когда сборку исполняет Конфигуратор; при `providers.push: ibcmd`
используйте `--sources`.

### `tools.va`

Поддержанные поля:

- `epf_path`, путь к внешней обработке Vanessa Automation.

`v8-runner tools download vanessa` заполняет `tools.va.epf_path` в `v8project.local.yaml` путём
`build/tools/vanessa-automation-single.epf`.

## `tools.platform`

### `tools.platform.path`

- Тип: путь
- Обязателен: нет

Может указывать:

- на конкретный бинарь `1cv8`, `1cv8c` или `ibcmd`;
- на каталог `bin`;
- на корень установки с версиями.

Относительный путь нормализуется относительно каталога primary `v8project.yaml`.
Если `path` задан, поиск platform utilities ограничивается этим путём и не переходит к default
roots или `PATH`, независимо от `strict`.

### `tools.platform.strict`

- Тип: boolean
- Обязателен: нет
- По умолчанию: `false`

`strict` управляет проверкой `tools.platform.version` внутри configured `path`. Сам `path` всегда
является explicit-only границей. При `strict: false` значение `version` для configured `path`
игнорируется. При `strict: true` найденная внутри `path` utility обязана соответствовать
`version`; неизвестная версия или несовпадение версии завершают команду ошибкой.

Если `path` указывает на конкретный executable, поиск sibling utilities (`1cv8`, `1cv8c`,
`ibcmd`) в `strict: false` идёт рядом с указанным файлом, а в `strict: true` — рядом с его
canonical installation. При `strict: true` первая найденная platform utility фиксирует один
canonical installation root; последующие `1cv8`, `1cv8c` и `ibcmd` выбираются только из этого root.

### `tools.platform.version`

- Тип: строка
- Обязателен: нет
- Формат: `major.minor`, `major.minor.patch` или `major.minor.patch.build`

Поведение:

- `8.3.27.1859`: требуется точное совпадение;
- `8.3.20`: выбирается максимальная найденная сборка `8.3.20.*`;
- `8.3`: выбирается максимальная найденная версия `8.3.*.*`.

Матрица поведения:

| Конфигурация | Поведение |
| --- | --- |
| `version`, без `path` | Поиск по default roots и `PATH` с проверкой версии. |
| `path + version`, `strict: false` | Поиск только по `path`; `version` игнорируется. |
| `path + version`, `strict: true` | Поиск только по `path`; версия обязана совпасть. |
| `path`, без `version` | Поиск только по `path`; проверки версии нет. |
| Без `path` и без `version` | Обычный поиск по default roots и `PATH`. |
| `strict: true`, без `path` | Не создаёт boundary; с `version` работает как version-only поиск, без `version` не меняет обычный поиск. |

Установка платформы бывает неполной, и это норма: тонкий клиент платформа доставляет отдельно и
обновляет сама под версию опубликованной базы, поэтому рядом с полной установкой живут каталоги
версий, где есть только `1cv8c`. Версия, подходящая под маску, может не содержать нужного команде
компонента.

Отказ на этот случай называет опись, а не только имя файла:

```text
utility '1cv8' was not found: version 8.5.1 is installed (8.5.1.1519, 8.5.1.1469) but has no full client; full client found in 8.5.4.1306, 8.3.27.2074
```

Компоненты в тексте отказа: `1cv8` — full client, `1cv8c` — thin client, `ibcmd` — server tools,
`webinst` — web server extensions. Опись строится по тому же пути, по которому шёл поиск: если задан
`path`, она описывает указанную установку, а не корни по умолчанию. Когда не нашлось ни одной
установки, отказ называет корни, в которых искал. У `1cedtcli` описи нет — EDT не компонент
платформы, и её отказ остаётся прежним.

## `tools.enterprise`

### `tools.enterprise.additional-launch-keys`

- Тип: список строк
- Обязателен: нет

Ключи добавляются к enterprise client launch.

## `tools.edt_cli`

### `tools.edt_cli.path`

- Тип: путь или version-like hint
- Обязателен: нет

Поддержанные варианты:

- абсолютный путь к `1cedtcli`;
- путь к каталогу установки EDT;
- version-like hint, например `2025.2.3`.

### `tools.edt_cli.version`

- Тип: строка
- Обязателен: нет

Отдельная подсказка для автопоиска EDT.

### `tools.edt_cli.interactive-mode`

- Тип: boolean
- По умолчанию: `false`

Переключает EDT execution между one-shot и shared interactive model.

### `tools.edt_cli.auto-start`

- Тип: boolean
- По умолчанию: `false`

Имеет эффект только вместе с `interactive-mode=true` и только для long-lived host process. На
текущем этапе это MCP server. CLI не делает eager prewarm и стартует EDT лениво при первом
EDT-вызове.

### `tools.edt_cli.startup_timeout_ms`

- Тип: integer
- По умолчанию: `300000`

### `tools.edt_cli.command_timeout_ms`

- Тип: integer
- По умолчанию: `300000`

## `tools.designer_agent`

Точка входа агента Конфигуратора для провайдера `agent`. Режим объявлен ключами: без
`attach` раннер поднимает агента сам (`managed`), с `attach` — подключается к агенту,
поднятому без него (`attached`), не добавляет ему флагов, не перезапускает и не
поднимает свой рядом. Ключи двух режимов не смешиваются: `attach` вместе с `port` или
`host-key` — ошибка валидации, `base-dir` без `attach` — тоже.

SSH-клиент встроен в раннер: внешний `ssh` не нужен ни на одной ОС. Сессия идёт без
псевдотерминала; учётные данные — `infobase.user` и `infobase.password`, у базы без
пользователей — пустая пара. Готовность агента доказывает успешная аутентификация, а не
открытый порт.

Ключ хоста сверяется, когда есть с чем. У управляемого агента — с открытой частью файла
`host-key`: агент публикует ключ оттуда как есть, поэтому раннер знает, что должен
увидеть. У чужой точки входа — с отпечатком, объявленным в
`tools.designer_agent.host-fingerprint` или `infobase.standalone.host-fingerprint`.
Несовпадение — типизированный отказ, называющий оба отпечатка.

Когда сверять не с чем — ключ принимается, а его отпечаток называется в предупреждении,
чтобы владелец мог закрепить его одной строкой. Так происходит у управляемого агента без
`host-key` (платформа берёт или создаёт свой, `/AgentSSHHostKeyAuto`, и раннер его не
знает), у управляемого с нечитаемым `host-key` (ключ мог быть под паролем, которого у
раннера нет) и у чужой точки входа без объявленного отпечатка.

Управляемый агент поднимается как `1cv8 DESIGNER <база> /AgentMode /AgentPort <port>
/AgentListenAddress 127.0.0.1 /AgentSSHHostKeyAuto /AgentBaseDir <workPath>/agent/base`,
поэтому для файловой и кластерной базы нужна локальная платформа. Агент читает и пишет
только внутри `AgentBaseDir`, поэтому каталоги проекта выставляются ему символической
ссылкой в каталоге пользователя агента (на Windows, где ссылку создать нельзя без
привилегии, каталог копируется). Журналы сессий — `workPath/logs/platform/build-agent.log`
и `workPath/logs/platform/dump-<набор>-agent.log`; учёт поколений конфигурации —
`workPath/agent/generation/<набор>.json`.

Секция целиком допустима в `v8project.local.yaml`: адрес чужого агента — свойство
машины.

### `tools.designer_agent.attach`

- Тип: строка `host:port` (IPv6 — в скобках: `[::1]:1543`)
- Обязателен: нет

Агент, поднятый вне раннера. Исключает `port` и `host-key`. Порт обязателен, как и у
`infobase.standalone.gate`: к этой точке идёт собственный SSH-клиент раннера, умолчания
платформы у него нет. Имя приводится к строчным, не-ASCII — к punycode, адрес IPv4 — к
каноническому виду; пробелы в записи — отказ.

### `tools.designer_agent.base-dir`

- Тип: путь
- Обязателен: только вместе с `attach` для операций, читающих результат с диска (`pull`)

`AgentBaseDir` чужого агента: относительно его пользовательского каталога агент
трактует пути команд.

### `tools.designer_agent.port`

- Тип: integer
- По умолчанию: `1543`

Порт управляемого агента.

### `tools.designer_agent.host-key`

- Тип: путь
- Обязателен: нет

Закрытый ключ хоста управляемого агента. Без него платформа берёт или создаёт свой
(`/AgentSSHHostKeyAuto`).

### `tools.designer_agent.host-fingerprint`

- Тип: строка
- Обязателен: нет

Отпечаток ключа хоста, который обязан предъявить чужой агент, в виде `SHA256:<base64>` —
то, что печатают `ssh-keyscan` и `ssh-keygen -lf`. Только для `attach`: у управляемого
агента ключ раннер отдаёт сам в `host-key`, и сверяет с ним. Не объявлен — ключ
принимается, а его отпечаток называется в предупреждении.

### `tools.designer_agent.startup_timeout_ms`

- Тип: integer
- По умолчанию: `120000`

Сколько ждать, пока управляемый агент примет первую аутентифицированную сессию.

## Неподдержанные ключи

### `tools.edt_cli.working-directory`

Текущий статус:

- не входит в supported config contract;
- подсвечивается JSON Schema как unsupported key;
- runtime loader отклоняет unsupported keys на YAML boundary;
- рабочий каталог EDT session сейчас фиксирован: `workPath/edt-workspace`.
