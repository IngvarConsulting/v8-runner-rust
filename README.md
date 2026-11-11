# v8-runner

> **Maintained fork.** This repository is developed and released by
> [Ingvar Consulting](https://github.com/IngvarConsulting). It was forked from
> [`alkoleft/v8-runner-rust`](https://github.com/alkoleft/v8-runner-rust) on
> 2026-09-02. See [FORK_NOTICE.md](FORK_NOTICE.md) for provenance, modification,
> source, and AGPL information.

`v8-runner` — командная строка для локального цикла разработки 1С. Она загружает исходники в
информационную базу, запускает проверки и тесты и выгружает изменения обратно в файлы. Вместо
хрупких скриптов и ручных запусков Конфигуратора, EDT и Vanessa Automation разработчик, CI и
AI-агент получают одну воспроизводимую точку входа.

## Зачем

- Быстрый цикл: `push` → `check`/`test` → разбор ошибок.
- Один файл проекта `v8project.yaml` — исходники, рабочая база, инструменты и тесты.
- Наборы исходников в форматах `DESIGNER` и `EDT`: основная конфигурация, расширения,
  внешние обработки и отчёты.
- Изменённые файлы находятся по содержимому — даже с прежним размером и прежним `mtime`.
- Исполнителя каждой операции — агента Конфигуратора, пакетный Конфигуратор или `ibcmd` —
  выбирает [матрица поддержки](docs/CAPABILITIES.md#матрица-поддержки); вручную его назначает
  ключ `providers.<операция>`.
- Машиночитаемый ответ `--json-message` для CI и агентов; с `--events -` ход долгой команды
  идёт событиями NDJSON до итогового конверта.
- Рабочий каталог `workPath` (по умолчанию `.build/v8-runner`) — логи, кеши и промежуточные
  файлы, его можно удалить; память о базе живёт в `localPath` (по умолчанию `.local`) и
  удаление `workPath` переживает.

![test-yaxunit](docs/assets/test-yaxunit.png)

## Установка

Каждый выпуск публикует по архиву на платформу: бинарник, этот README, лицензия, уведомление
о форке и `examples/`.

| Архив | Система | Процессор |
| --- | --- | --- |
| `v8-runner-linux-x86_64-musl.tar.gz` | Linux, любой дистрибутив (статическая сборка musl) | Intel/AMD 64 |
| `v8-runner-macos-aarch64.tar.gz` | macOS | Apple Silicon (M1 и новее) |
| `v8-runner-macos-x86_64.tar.gz` | macOS | Intel |
| `v8-runner-windows-x86_64.zip` | Windows | Intel/AMD 64 |

Проверить происхождение выпуска — `gh release verify <tag> --repo IngvarConsulting/v8-runner-rust`;
проверка отдельного архива по манифесту и аттестациям, offline-проверка и история имён файлов —
[docs/RELEASES.md](https://github.com/IngvarConsulting/v8-runner-rust/blob/master/docs/RELEASES.md).

Сборка из исходников — `cargo build --release`, бинарник ляжет в `target/release/v8-runner`.

Раннер поддерживает платформу 1С 8.3.24 и новее
([минимальная версия](docs/CAPABILITIES.md#минимальная-версия-платформы)). `v8-runner platforms`
показывает найденные установки и какую из них возьмёт каждая утилита: без
`tools.platform.version` — выпуск, который пишет формат исходников проекта
([выбор платформы](docs/CONFIGURATION.md#toolsplatformversion)).

## Первый цикл

1. **Проект.** `v8-runner init` — находит наборы исходников и пишет `v8project.yaml` и местный
   слой `v8project.local.yaml` с базой `origin` (по умолчанию `File=.local/db`); что ещё он
   дописывает в `.gitignore` — [CAPABILITIES](docs/CAPABILITIES.md#init).
   Из существующей базы вместо этого:
   `v8-runner clone --from "File=/path/to/ib" --platform-version 8.3.27` — проект и выгрузка
   основной конфигурации в `src/configuration`.
2. **Инструменты тестов.** `v8-runner tools download yaxunit --sources` и
   `v8-runner tools download vanessa` — последний выпуск YAxUnit и Vanessa Automation
   ([подробнее](docs/CAPABILITIES.md#tools-download)).
3. **База.** `v8-runner infobase create` — рабочая база по виду цели; копия другой объявленной
   базы с данными — `--from <база>`.
4. **Загрузка.** `v8-runner push` — загружает изменённые исходники и применяет конфигурацию к
   базе данных; `--full` грузит набор целиком.
5. **Проверка.** `v8-runner check` — проверка конфигурации Конфигуратором или проекта EDT.
6. **Тесты.** `v8-runner test yaxunit all` или `v8-runner test va` — сначала `push`, затем
   тесты; `--no-push` — на уже подготовленной базе.
7. **Обратно в файлы.** `v8-runner pull` — выгружает изменения, сделанные в базе, в исходники.
8. **Состояние.** `v8-runner status` — как связаны каталог и база, по памяти и без запуска
   платформы; `--deep` спрашивает саму базу.

Базы проекта объявляются в `v8project.local.yaml` картой `infobases`; другую базу выбирает
`--infobase <имя|строка соединения>`. Местный слой хранит пути машины и учётные данные и должен
оставаться вне Git ([CONFIGURATION](docs/CONFIGURATION.md#infobases)). Если `init` не покрывает
структуру репозитория, `v8project.yaml` настраивается вручную по
[docs/CONFIGURATION.md](docs/CONFIGURATION.md).

Почти у каждой команды, которая запускает платформу, есть превью `--dry-run`: план и
выбранный исполнитель без запуска и без создания файлов
([какие команды](docs/CAPABILITIES.md#превью-у-глаголов-работающих-с-платформой)).

## Что умеет

| Зона | Команды | Что делает |
| --- | --- | --- |
| Настройка проекта | `init`, `clone`, `tools download`, `infobase create`, `extensions` | Создаёт проект, скачивает инструменты, готовит базу, настраивает расширения |
| Изменения в базе | `push`, `apply`, `reset` | Загружает исходники, применяет конфигурацию к базе данных или отбрасывает непринятое |
| Состояние | `status`, `diff`, `platforms` | Связь каталога и базы, изменения версий объектов, найденные платформы |
| Проверка | `check`, `test` | Проверка конфигурации, YAxUnit и Vanessa Automation |
| Файлы и пакеты | `pull`, `download`, `convert`, `upload`, `make` / `artifacts` | Выгружает исходники, собирает, конвертирует и загружает `.cf`, `.cfe`, `.epf`, `.erf` |
| Образ базы | `infobase dump`, `infobase restore` | Сохраняет базу с данными в `.dt` и загружает из него |
| Стенд | `dbms`, `standalone`, `cluster`, `agent`, `publish` | PostgreSQL, автономный сервер, кластер и агент Конфигуратора раннера, веб-публикация |
| Прямой запуск | `launch <designer\|thin\|thick\|ordinary>` | Запускает Конфигуратор и клиенты 1С |

Полный каталог команд, исполнители, коды отказов и ограничения —
[docs/CAPABILITIES.md](docs/CAPABILITIES.md). Прежние имена команд и ключей принимаются один
цикл выпуска — [перечень](docs/CAPABILITIES.md#прежние-имена-команд-и-ключей).

## Документация

- [docs/CAPABILITIES.md](docs/CAPABILITIES.md) — команды, матрица поддержки, отказы,
  несовместимые изменения, текущие ограничения; там же
  [MCP-сервер](docs/CAPABILITIES.md#mcp-serve).
- [docs/CONFIGURATION.md](docs/CONFIGURATION.md) — контракт `v8project.yaml` и местного слоя,
  ключи и правила проверки.
- [docs/DEEP_DIVE.md](docs/DEEP_DIVE.md) — как идёт выполнение: блокировки, публикация,
  память о базе.
- [docs/RELEASES.md](docs/RELEASES.md) — архивы выпуска и проверка их происхождения.
- [SKILL/SKILL.md](SKILL/SKILL.md) — навык для агента, который работает с `v8-runner` в
  проекте 1С.
- [docs/README.md](docs/README.md) — какой источник на что отвечает: код, правила, описания.
- [spec/arc42/](spec/arc42/architecture.md) — устройство: карта модулей (раздел 5), потоки,
  сквозные механизмы; для контрибьюторов.
- [spec/README.md](spec/README.md) — внутренний слой: правила продукта и архитектурное
  описание.
- [references/1c/README.md](references/1c/README.md) — сырой внешний корпус справочных
  материалов по 1С, не источник истины проекта.
