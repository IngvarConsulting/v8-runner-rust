# v8-runner

> **Maintained fork.** This repository is developed and released by
> [Ingvar Consulting](https://github.com/IngvarConsulting). It was forked from
> [`alkoleft/v8-runner-rust`](https://github.com/alkoleft/v8-runner-rust) on
> 2026-09-02. See [FORK_NOTICE.md](FORK_NOTICE.md) for provenance, modification,
> source, and AGPL information.

`v8-runner` — CLI (командная строка) и MCP server (сервер Model Context Protocol) для
локального 1C development workflow (цикла разработки 1С). Он собирает исходники, готовит
информационную базу, запускает проверки и тесты, выгружает изменения обратно в файлы и дает
AI-агентам безопасную, уже ограниченную MCP-поверхность.

Проект закрывает практическую боль 1С-разработки: вместо набора хрупких shell scripts
(скриптов оболочки), ручных запусков Designer (Конфигуратора), EDT и Vanessa Automation команда
получает один воспроизводимый entrypoint (точку входа) для локального цикла и автоматизации.

## Зачем это нужно

- Быстрый feedback loop (цикл обратной связи): `push -> check/test -> diagnose`.
- Один config (конфиг) `v8project.yaml` для исходников, рабочей ИБ, инструментов и тестов.
- Поддержка source sets (наборов исходников) в форматах `DESIGNER` и `EDT`.
- Исполнителя каждой операции выбирает матрица возможностей — Конфигуратор или `ibcmd` там, где это разрешает контракт 1С; вручную назначается ключом `providers.<операция>`.
- Machine-readable output (машиночитаемый вывод) через `--json-message` для CI и агентов.
- MCP tools (MCP-инструменты) для управляемой работы AI-агентов без выдачи всей CLI-поверхности.
- Изолированный `workPath` для hash storages (хранилищ хэшей), логов, временных файлов и
  промежуточных артефактов.

![test-yaxunit](docs/assets/test-yaxunit.png)

## Быстрый старт

Соберите release binary (релизный бинарный файл):

```bash
cargo build --release
```

Команда компилирует `v8-runner` в `target/release/v8-runner`.

### Release assets

Каждый выпуск публикует **один архив на платформу**. Внутри — бинарник `v8-runner`
(на Windows `v8-runner.exe`), этот README, лицензия, уведомление о форке и каталог
`examples/`:

| Архив | Система | Процессор |
| --- | --- | --- |
| `v8-runner-linux-x86_64-musl.tar.gz` | Linux, любой дистрибутив (статическая сборка musl) | Intel/AMD 64 |
| `v8-runner-macos-aarch64.tar.gz` | macOS | Apple Silicon (M1 и новее) |
| `v8-runner-macos-x86_64.tar.gz` | macOS | Intel |
| `v8-runner-windows-x86_64.zip` | Windows | Intel/AMD 64 |

Отличить Apple Silicon от Intel: `uname -m` отвечает `arm64` или `x86_64`.

Единый `v8-runner-assets.json` schema v2 связывает исходные tag/commit с ролью,
target, размером и SHA-256 всех остальных assets, а для архивов — также с путём и
SHA-256 вложенного бинарника. Все payload assets и manifest имеют GitHub build
attestations, которые подтверждают их происхождение;
`license-v8-runner-AGPL-3.0-only.txt` и `notice-v8-runner-fork.txt` лежат рядом и
также входят в manifest. Corresponding Source — неизменяемый tag того же release.

До `v0.11.0` включительно рядом с архивами выкладывались несжатые бинарники
`v8-runner-darwin-arm64`, `v8-runner-linux-x64` и `v8-runner-win-x64.exe`. Это были
байт в байт те же файлы, что лежат в архивах, и одна платформа выходила под двумя
именами. Они больше не публикуются.

В `v0.6.x` публиковались отдельные `.sha256` и `.provenance.json`. С `v0.7.0`
их заменяет единый manifest; имена архивов и юридических файлов при этом не
менялись.

Перед использованием проверьте release и конкретный бинарник:

```bash
gh release verify v0.7.0 --repo IngvarConsulting/v8-runner-rust
gh release download v0.7.0 --repo IngvarConsulting/v8-runner-rust \
  --pattern v8-runner-assets.json --pattern v8-runner-linux-x86_64-musl.tar.gz
gh release verify-asset v0.7.0 ./v8-runner-assets.json \
  --repo IngvarConsulting/v8-runner-rust
gh release verify-asset v0.7.0 ./v8-runner-linux-x86_64-musl.tar.gz \
  --repo IngvarConsulting/v8-runner-rust
source_commit="$(python3 -c 'import json; print(json.load(open("v8-runner-assets.json"))["release"]["sourceCommit"])')"
for asset in v8-runner-assets.json v8-runner-linux-x86_64-musl.tar.gz; do
  gh attestation verify "$asset" \
    --repo IngvarConsulting/v8-runner-rust \
    --signer-workflow IngvarConsulting/v8-runner-rust/.github/workflows/release.yml \
    --source-ref refs/heads/master --source-digest "$source_commit" \
    --deny-self-hosted-runners
done
```

Для offline-проверки сначала проверьте и перенесите в изолированную среду
`v8-runner-assets.json`, затем сравните SHA-256 нужного файла с соответствующей
записью manifest. Manifest, скачанный вместе с бинарником без предварительной
проверки `gh release verify-asset`, сам по себе не является корнем доверия.

### Создайте стартовый config (конфиг) в текущем репозитории:

```bash
v8-runner init
```

Команда анализирует структуру проекта, находит поддержанные `source-set` (наборы исходников),
создает `v8project.yaml`, `v8project.local.yaml` со schema modeline и базой `origin`
(`--connection`, по умолчанию `File=build/ib`) и добавляет local overlay в `.gitignore`, если
он еще не указан.

Базы проекта объявляются в `v8project.local.yaml` картой `infobases`: умолчание — `origin`,
другую выбирает `--infobase <имя|строка соединения>`. Там же живут machine-local пути,
credentials и настройки инструментов. Файл применяется автоматически и должен оставаться вне Git.

### Или создайте проект из существующей информационной базы:

```bash
v8-runner clone \
  --connection "File=/path/to/ib" \
  --platform-version 8.3.27
```

Команда создает `v8project.yaml`, локальный overlay, `.gitignore` и выгружает основную
конфигурацию в `src/configuration`. Адрес базы и credentials (`--user`, `--password`)
попадают только в `v8project.local.yaml`, в секцию `infobases.origin`. Автоматическое обнаружение расширений этот `clone`
slice не выполняет.

### Загрузите тестовые и MCP-инструменты:

```bash
v8-runner tools download yaxunit --sources
v8-runner tools download vanessa
v8-runner tools download client-mcp --sources
```

Команды берут latest releases выбранного инструмента. Для YAxUnit и onec-client-mcp-devkit
`--sources` выбирает source install; без него скачивается `.cfe` artifact в `build/tools`.
Vanessa Automation single всегда скачивается как EPF в `build/tools` и прописывается в
`v8project.local.yaml`.

### Подготовьте рабочую информационную базу:

```bash
v8-runner infobase create
```

Команда создает или подготавливает ИБ и, для `EDT`, импортирует workspace (рабочую область).

### Загрузите исходники в ИБ:

```bash
v8-runner push
```

Команда выполняет incremental build (инкрементальную сборку) или full path (полную сборку) по
текущим изменениям и настройкам проекта.

### Спланируйте или выгрузите состояние ИБ:

```bash
v8-runner download --state working --output dist/main.cf --dry-run
v8-runner infobase dump --output dist/base.dt --dry-run
```

`--dry-run` валидирует окружение и показывает выбранный provider без запуска платформы и без
создания файлов. Уберите флаг, чтобы атомарно опубликовать CF/CFE или переносимый DT-файл.
Ключ глобальный — его место в строке не важно, — а команда без превью (`version`, `init`,
`tools download`, `test`, `mcp serve`) отвергает его с названной причиной.

Обратная операция загружает ИБ из DT-файла:

```bash
v8-runner infobase restore --input dist/base.dt --replace --dry-run
v8-runner infobase restore --input dist/base.dt --create
```

Ровно один режим цели обязателен: `--replace` отбрасывает данные существующей ИБ, `--create`
создаёт отсутствующую. Режим, не совпавший с наблюдаемой целью, отклоняется до запуска
платформы, потому что отменить загрузку нечем — staging-шага, в отличие от выгрузки, здесь нет.

### Проверьте синтаксис серверных модулей:

```bash
v8-runner check --server
```

Команда запускает Designer syntax check (проверку синтаксиса Конфигуратором) для серверного
контекста.

### Запустите YAxUnit-тесты:

```bash
v8-runner test yaxunit all
```

Для уже подготовленной файловой или серверной ИБ можно явно пропустить `push`:

```bash
v8-runner test --no-push yaxunit all
```

Для файловой ИБ этот режим до запуска 1С проверяет наличие `1Cv8.1CD`.
Проверка конфигурации не требует наличия project source-set: нужны только настройки ИБ,
платформы и выбранного test engine. Для server connection отдельный portable preflight без
запуска платформы пока недоступен, поэтому соединение проверяет сам test engine.

### Или тесты Vanessa Automation:

```bash
v8-runner test va
```

По умолчанию команда сначала выполняет `push`, затем запускает настроенный профиль Vanessa
Automation. Для подготовленной ИБ используйте `v8-runner test --no-push va`.

Для отладки и написания тестов Vanessa Automation запустите ее в режиме MCP и, если агенту нужно
сразу подключаться к endpoint, дождитесь готовности:

```bash
v8-runner launch mcp va --mcp-port 1550 --wait-ready
```

Для функциональных `.feature`-сценариев, приемки и задач Vanessa Automation используйте
`test va`, MCP `run_all_tests` с `runner=vanessa` или `launch mcp va --wait-ready`; голый
`launch mcp` предназначен только для client MCP без загрузки Vanessa.

Для автоматизации `v8-runner --json-message launch ...` сохраняет поле `binary` и добавляет
canonical `platform_resolution` (path, version, source и installation root). Эта metadata
публикуется только для результата `launch`, а не для всех команд.

Чтобы узнать, что именно будет запущено, не запуская клиент:

```bash
v8-runner --json-message launch thin --dry-run
```

Превью возвращает `provider_dispatched=false`, `pid=null` и `plan` с выбранной программой и уже
составленными аргументами; значения credential в них замаскированы как `***`. Уберите флаг, чтобы
запустить клиент.

### Поднимите MCP transport (MCP-транспорт) для AI-агентов:

```bash
v8-runner mcp serve stdio
```

Команда запускает MCP server (сервер Model Context Protocol) поверх `stdio` transport
(транспорта стандартного ввода-вывода).

Если `init` не покрывает вашу структуру репозитория, настройте `v8project.yaml` вручную по
[docs/CONFIGURATION.md](docs/CONFIGURATION.md).

## Что умеет

| Зона | Команды | Что делает |
| --- | --- | --- |
| Project setup (настройка проекта) | `clone`, `init`, `tools download`, `infobase create`, `extensions`, `push` | Создает проект/config, скачивает инструменты, готовит ИБ, обновляет расширения и загружает исходники |
| Verification (проверка) | `check`, `test` | Запускает syntax checks, YAxUnit и Vanessa Automation |
| File materialization (материализация файлов) | `pull`, `download`, `convert`, `upload`, `make`, `artifacts` | Выгружает, конвертирует, загружает и публикует `.cf`, `.cfe`, `.epf`, `.erf` |
| Direct launch (прямой запуск) | `launch <designer\|thin\|thick\|ordinary>`, `launch mcp [va]` | Запускает 1C clients (клиенты 1С), Designer и MCP/Vanessa сценарии |
| MCP automation (автоматизация через MCP) | `mcp serve stdio`, `mcp serve http` | Открывает 8 MCP tools для агентных workflow |

Команды названы словарём гита. Прежние имена приняты ещё один цикл выпуска и в справке не
печатаются: `bootstrap` → `clone`, `config init` → `init`, `build` → `push`, `load` → `upload`,
`dump` → `pull`, `syntax` → `check`; прежний путь `infobase configuration export` тоже
принимается. То же с ключами: `--full-rebuild` → `--full`, `--discard-uncommitted` → `--force`,
`--no-build` → `--no-push`, `--mode merge` → `--mode combine`. Ответ приходит под новым именем.
Создание базы синонима не имеет: имя `init` занято подготовкой проекта, база создаётся командой
`infobase create`.

## Для кого

- 1С-разработчики, которым нужен повторяемый локальный цикл без ручного переключения между
  Designer, EDT, Vanessa Automation и тестовыми runner-ами.
- Команды, которые хотят единый command contract (контракт команд) для локальной разработки,
  CI и релизной сборки.
- AI-assisted development (разработка с AI-агентами), где агент должен строить, проверять и
  диагностировать проект через узкую управляемую поверхность.

Настроить безопасность отдельно установленного CFE, например YaXUnit:

```bash
v8-runner extensions --installed-name YAXUNIT --dry-run
v8-runner extensions --installed-name YAXUNIT
```

Применение отключает безопасный режим и защиту от опасных действий. Имя не требует
соответствующего `source-set`; для совместного выбора добавьте `--name TESTS`.

## Карта документации

- [docs/CAPABILITIES.md](docs/CAPABILITIES.md): полный каталог команд, матрица поддержки,
  MCP tools и текущие ограничения.
- [docs/CONFIGURATION.md](docs/CONFIGURATION.md): контракт `v8project.yaml`, поддержанные keys
  (ключи) и validation rules (правила валидации).
- [docs/DEEP_DIVE.md](docs/DEEP_DIVE.md): execution semantics (семантика выполнения), runtime
  model (модель выполнения), lock/publication behavior (поведение блокировок и публикации).
- [docs/README.md](docs/README.md): какой источник на что отвечает — код, правила,
  описания.
- [spec/arc42/](spec/arc42/architecture.md): устройство — карта модулей (раздел 5), потоки,
  сквозные механизмы; для контрибьюторов.
- [spec/README.md](spec/README.md): внутренний слой — правила продукта (architecture rules)
  и архитектурное описание.
- [references/1c/README.md](references/1c/README.md): сырой внешний reference corpus
  (корпус справочных материалов) по 1С, не source of truth проекта.
