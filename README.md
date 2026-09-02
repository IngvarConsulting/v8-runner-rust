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

- Быстрый feedback loop (цикл обратной связи): `build -> syntax/test -> diagnose`.
- Один config (конфиг) `v8project.yaml` для исходников, рабочей ИБ, инструментов и тестов.
- Поддержка source sets (наборов исходников) в форматах `DESIGNER` и `EDT`.
- Builder backends (сборщики) `DESIGNER` и `IBCMD` там, где это разрешает контракт 1С.
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

Каждый выпуск сохраняет portable `.tar.gz`/`.zip` archives для ручной установки
и публикует из тех же matrix builds три готовых бинарника для Unica:
`v8-runner-darwin-arm64`, `v8-runner-linux-x64` и
`v8-runner-win-x64.exe`. Для них выпуск содержит SHA-256, компактный provenance
manifest и GitHub build attestations; `license-v8-runner-AGPL-3.0-only.txt` и
`notice-v8-runner-fork.txt` лежат рядом. Corresponding Source — неизменяемый tag
того же release.

Перед использованием проверьте release и конкретный бинарник:

```bash
gh release verify v0.5.2-ic.3 --repo IngvarConsulting/v8-runner-rust
gh attestation verify v8-runner-linux-x64 \
  --repo IngvarConsulting/v8-runner-rust \
  --signer-workflow IngvarConsulting/v8-runner-rust/.github/workflows/release.yml \
  --source-ref refs/heads/master --deny-self-hosted-runners
```

### Создайте стартовый config (конфиг) в текущем репозитории:

```bash
v8-runner config init
```

Команда анализирует структуру проекта, находит поддержанные `source-set` (наборы исходников),
создает `v8project.yaml`, пустой `v8project.local.yaml` со schema modeline и добавляет local
overlay в `.gitignore`, если он еще не указан.

Machine-local пути, credentials и настройки инструментов можно вынести в `v8project.local.yaml`
рядом с основным конфигом. Этот файл применяется автоматически и должен оставаться вне Git.

### Или создайте проект из существующей информационной базы:

```bash
v8-runner bootstrap \
  --connection "File=/path/to/ib" \
  --platform-version 8.3.27
```

Команда создает `v8project.yaml`, локальный overlay, `.gitignore` и выгружает основную
конфигурацию в `src/configuration`. Credentials передавайте через `--user` и `--password`; они
попадают только в `v8project.local.yaml`. Автоматическое обнаружение расширений этим bootstrap
slice не выполняется.

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
v8-runner init
```

Команда создает или подготавливает ИБ и, для `EDT`, импортирует workspace (рабочую область).

### Загрузите исходники в ИБ:

```bash
v8-runner build
```

Команда выполняет incremental build (инкрементальную сборку) или full path (полную сборку) по
текущим изменениям и настройкам проекта.

### Проверьте синтаксис серверных модулей:

```bash
v8-runner syntax designer-modules --server
```

Команда запускает Designer syntax check (проверку синтаксиса Конфигуратором) для серверного
контекста.

### Запустите YAxUnit-тесты:

```bash
v8-runner test yaxunit all
```

Для уже подготовленной файловой или серверной ИБ можно явно пропустить build:

```bash
v8-runner test --no-build yaxunit all
```

Для файловой ИБ этот режим до запуска 1С проверяет наличие `1Cv8.1CD`.
Проверка конфигурации не требует наличия project source-set: нужны только настройки ИБ,
платформы и выбранного test engine. Для server connection отдельный portable preflight без
запуска платформы пока недоступен, поэтому соединение проверяет сам test engine.

### Или тесты Vanessa Automation:

```bash
v8-runner test va
```

По умолчанию команда сначала выполняет `build`, затем запускает настроенный профиль Vanessa
Automation. Для подготовленной ИБ используйте `v8-runner test --no-build va`.

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

### Поднимите MCP transport (MCP-транспорт) для AI-агентов:

```bash
v8-runner mcp serve stdio
```

Команда запускает MCP server (сервер Model Context Protocol) поверх `stdio` transport
(транспорта стандартного ввода-вывода).

Если `config init` не покрывает вашу структуру репозитория, настройте `v8project.yaml` вручную по
[docs/CONFIGURATION.md](docs/CONFIGURATION.md).

## Что умеет

| Зона | Команды | Что делает |
| --- | --- | --- |
| Project setup (настройка проекта) | `bootstrap`, `config init`, `tools download`, `init`, `extensions`, `build` | Создает проект/config, скачивает инструменты, готовит ИБ, обновляет расширения и загружает исходники |
| Verification (проверка) | `syntax`, `test` | Запускает syntax checks, YAxUnit и Vanessa Automation |
| File materialization (материализация файлов) | `dump`, `convert`, `load`, `make`, `artifacts` | Выгружает, конвертирует, загружает и публикует `.cf`, `.cfe`, `.epf`, `.erf` |
| Direct launch (прямой запуск) | `launch <designer|thin|thick|ordinary>`, `launch mcp [va]` | Запускает 1C clients (клиенты 1С), Designer и MCP/Vanessa сценарии |
| MCP automation (автоматизация через MCP) | `mcp serve stdio`, `mcp serve http` | Открывает 8 MCP tools для агентных workflow |

## Для кого

- 1С-разработчики, которым нужен повторяемый локальный цикл без ручного переключения между
  Designer, EDT, Vanessa Automation и тестовыми runner-ами.
- Команды, которые хотят единый command contract (контракт команд) для локальной разработки,
  CI и релизной сборки.
- AI-assisted development (разработка с AI-агентами), где агент должен строить, проверять и
  диагностировать проект через узкую управляемую поверхность.

## Карта документации

- [docs/CAPABILITIES.md](docs/CAPABILITIES.md): полный каталог команд, матрица поддержки,
  MCP tools и текущие ограничения.
- [docs/CONFIGURATION.md](docs/CONFIGURATION.md): контракт `v8project.yaml`, поддержанные keys
  (ключи) и validation rules (правила валидации).
- [docs/DEEP_DIVE.md](docs/DEEP_DIVE.md): execution semantics (семантика выполнения), runtime
  model (модель выполнения), lock/publication behavior (поведение блокировок и публикации).
- [docs/README.md](docs/README.md): порядок чтения документации и source-of-truth (источник
  истины).
- [ARCHITECTURE.md](ARCHITECTURE.md): module map (карта модулей) и границы для контрибьюторов.
- [spec/README.md](spec/README.md): внутренние ADR, architecture rules (архитектурные правила),
  acceptance (приемка) и implementation backlog (план реализации).
- [references/1c/README.md](references/1c/README.md): сырой внешний reference corpus
  (корпус справочных материалов) по 1С, не source of truth проекта.
