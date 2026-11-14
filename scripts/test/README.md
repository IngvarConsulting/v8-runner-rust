# `scripts/test`

Набор helper-скриптов для локального smoke/UAT запуска и для CI entrypoint-слоя вокруг `v8-runner`.

Здесь разделены четыре роли:

- CI orchestration: выбор и запуск нужного контура проверки.
- Live CLI smoke: запуск реального сценария через CLI `v8-runner`.
- UAT wrappers: запуск "с нуля" с подготовкой бинаря и очисткой артефактов.
- MCP smoke: проверка HTTP-поднятия MCP сервера и вызовов MCP tools.

## Карта вызовов

```text
.github/workflows/ci.yml
  -> ci-platform-install.sh
  -> ci-designer-config.sh
  -> ci-rust.sh
    -> ci-happy-path.sh
      -> live-cli-smoke-test.sh
      -> live-cli-fixture.sh

ci-rust.sh
  -> ci-happy-path.sh
    -> live-cli-smoke-test.sh
    -> live-cli-fixture.sh

live-cli-designer.sh
  -> live-cli-fixture.sh

live-cli-ibcmd.sh
  -> live-cli-fixture.sh

uat-cli-ibcmd.sh
  -> live-cli-ibcmd.sh
    -> live-cli-fixture.sh

live-mcp-http.py
  -> v8-runner mcp serve http
  -> MCP initialize/tools/list/tools/call smoke
```

## Скрипты

| Файл | Роль | Назначение | Зона ответственности |
| --- | --- | --- | --- |
| `ci-rust.sh` | CI entrypoint | Диспетчер CI-контуров по `V8_RUNNER_CI_SCOPE` | Выбрать нужный scope и передать управление в Linux/macOS `cargo test`, Windows contract или `ci-happy-path.sh` |
| `ci-happy-path.sh` | CI helper | Canonical happy-path для trusted CI | Собрать бинарь, выполнить `cargo check`, опционально `cargo test`, затем запустить обязательный packaging/live contour |
| `ci-platform-install.sh` | CI helper | Установить 1С platform bundle на GitHub-hosted runner | Скачать secret-backed bundle, проверить checksum, распаковать и отдать `tools.platform.path`/`ibsrv` paths |
| `ci-designer-config.sh` | CI helper | Подготовить конфиг для обязательного smoke в CI | Заполнить `providers`, подключение к файловой ИБ и `tools.platform.path` |
| `ci-ibsrv.sh` | Вспомогательный скрипт | Запустить или остановить standalone `ibsrv` вручную | Передать серверу `--data` и `--db-path` существующей базы |
| `live-cli-smoke-test.sh` | Проверка без 1С | Проверить три сгенерированных конфига и запись квитанций | Вызвать обе обёртки и генератор CI, проверить конфиги текущим бинарём, проверить повторный запуск и код выхода команды |
| `live-cli-fixture.sh` | Общий harness | Выполнить smoke на штатных исходниках в формате Designer | Развернуть рабочую копию, выполнить `infobase create`, `push`, `extensions` и последующие проверки |
| `live-cli-designer.sh` | Live entrypoint | Запустить smoke Конфигуратора | Подготовить конфиг и передать его в `live-cli-fixture.sh` |
| `live-cli-ibcmd.sh` | Live entrypoint | Запустить smoke IBCMD | Подготовить конфиг без внешних наборов и передать его в `live-cli-fixture.sh` |
| `uat-cli-ibcmd.sh` | UAT wrapper | Полный запуск IBCMD smoke "с нуля" | Собрать бинарь, очистить старые артефакты и вызвать `live-cli-ibcmd.sh` |
| `live-mcp-http.py` | MCP smoke | Live-проверка MCP HTTP сервера | Поднять `mcp serve http`, выполнить `initialize`, `tools/list` и `tools/call` smoke-последовательность |
| `../site_matrix.py` | CI check | Сверка «сайт = матрица» (#225), шаг Happy Path | Сверить цепочки `docs/site/data.js` и таблицу `architecture.html#d-ops` с `docs/schemas/capability-matrix.json`; порядок работы и перепорождение артефакта (`UPDATE_CAPABILITY_MATRIX=1 cargo test --bin v8-runner generated_capability_matrix_is_current`) — [`docs/site/README.md`, раздел «Сверка с матрицей»](../../docs/site/README.md#сверка-с-матрицей) |

## Границы ответственности

### `ci-rust.sh`

- Отвечает только за выбор CI-scope.
- Windows contract scope дополнительно запускает нативный CLI smoke выгрузок CFE и DT через
  test-only platform fixture; это сохраняет end-to-end проверку маршрутизации, staging и
  публикации на Windows без требования установленной платформы 1С. Тот же scope запускает
  процессные тесты внешней публикации полного `pull`, cross-process contention smoke
  и unit-регрессии общего OS/legacy lock protocol.
- Linux и macOS contract scope запускают полный `cargo test --locked`; неизвестный OS label
  считается ошибкой конфигурации CI, а не неявным Linux/macOS fallback.
- Не знает деталей live fixture и не должен дублировать live smoke-логику.

### `ci-happy-path.sh`

- Отвечает за canonical blocking helper chain для happy-path.
- Должен собирать Rust-бинарь и запускать обязательные Rust-проверки до live contour.
- Не должен дублировать шаги `init/build/make/dump`; за них отвечает `live-cli-fixture.sh`.

### `ci-platform-install.sh`

- Используется только в GitHub Actions trusted live path.
- Принимает OS-specific secret URL bundle-а и обязательный SHA256, затем извлекает из bundle минимум `1cv8` и `ibsrv`.
- Не должен знать про `build/test/make`; его задача заканчивается на установке platform root/hint и путей до утилит.

### `ci-designer-config.sh`

- Используется в trusted live CI и в проверке материализации без 1С.
- Материализует explicit `V8TR_DESIGNER_REAL_CONFIG`, чтобы mandatory smoke не зависел от implicit fallback.
- Хранит конфиг и путь к новой файловой ИБ в выделенном runtime-каталоге. Базу создаёт `infobase create` в начале smoke.

### `ci-ibsrv.sh`

- Поднимает standalone `ibsrv` для существующей файловой ИБ. Designer smoke в CI обращается к своей базе напрямую.

### `live-cli-fixture.sh`

- Это главный исполнитель реального fixture-based smoke.
- Он владеет порядком шагов, валидацией config, подготовкой workspace и проверкой результата.
- В режиме Designer выполняет `check`, опциональный `test` и проверяет упаковку `.cf/.cfe/.epf/.erf`.
- В режиме IBCMD выполняет полную, инкрементальную и пообъектную команды `pull`.
- При каждом запуске очищает выделенный output root и заново создаёт каталог `json`. У стандартных ручных сценариев файловая ИБ также находится в этом root.
- Именно этот скрипт является общим contract layer для `live-cli-designer.sh` и `live-cli-ibcmd.sh`.

### `live-cli-designer.sh`

- Это тонкая обёртка над `live-cli-fixture.sh`.
- Если `V8TR_DESIGNER_REAL_CONFIG` не задан, скрипт сам материализует временный config из `live-cli-designer.fixture.yaml`.
- Не должен повторять шаги smoke-harness; его зона ответственности заканчивается на подготовке designer-config и env.

### `live-cli-ibcmd.sh`

- Это тонкая IBCMD-специализация поверх общего fixture harness.
- Если `V8TR_IBCMD_REAL_CONFIG` не задан, скрипт материализует временный config на основе `live-cli-designer.fixture.yaml`.
- Скрипт назначает `ibcmd` в `providers` и удаляет внешние наборы `EXTERNAL_DATA_PROCESSORS` и `EXTERNAL_REPORTS` из сгенерированного конфига.
- Не должен заниматься сборкой бинаря и общей очисткой стенда; это ответственность UAT wrapper или вызывающего окружения.

### `uat-cli-ibcmd.sh`

- Это сценарий "запустить IBCMD UAT с чистого листа".
- Он отвечает за `cargo build`, удаление `target/manual-tests/live-cli-ibcmd` и `target/manual-tests/live-cli-ibcmd.generated.yaml`, а затем за запуск `live-cli-ibcmd.sh`.
- Он не должен дублировать материализацию IBCMD-config или шаги smoke-runner.

### `live-mcp-http.py`

- Отвечает за smoke-проверку MCP HTTP protocol path.
- Использует реальный config из `V8TR_REAL_CONFIG`, поднимает сервер, ждёт порт, затем как клиент вызывает MCP lifecycle и tools.
- Не заменяет CLI smoke; это отдельный контур, который проверяет именно MCP transport и tool contract.

## Вспомогательные файлы

| Файл | Назначение | Ответственность |
| --- | --- | --- |
| `live-cli-designer.fixture.yaml` | Базовый шаблон fixture-based live config | Описывает `format`, `providers`, `infobase`, `source-set` и placeholders для изолированного запуска |
| `live-cli-designer.va-params.json` | Шаблон параметров Vanessa Automation | Используется в opt-in `V8TR_DESIGNER_TEST_MODE=va` |
| `features/live-cli-designer/smoke.feature` | Минимальный VA smoke-feature | Держит простой сценарий для проверки интеграции VA |

## Основные переменные окружения

| Переменная | Где используется | Назначение |
| --- | --- | --- |
| `V8_RUNNER_CI_SCOPE` | `ci-rust.sh` | Выбор CI-контура: `contract`, `full`, `runtime-locks`, `happy-path` |
| `V8TR_BIN` | Почти все entrypoint-скрипты | Явный путь к бинарю `v8-runner` |
| `V8TR_PLATFORM_BUNDLE_URL` | `ci-platform-install.sh` | Secret-backed URL platform bundle для текущей ОС |
| `V8TR_PLATFORM_BUNDLE_SHA256` | `ci-platform-install.sh` | Обязательный SHA256 platform bundle для trusted install path |
| `V8TR_DESIGNER_REAL_CONFIG` | `live-cli-fixture.sh`, `live-cli-designer.sh` | Явный live-config для designer contour |
| `V8TR_IBCMD_REAL_CONFIG` | `live-cli-ibcmd.sh` | Явный live-config для IBCMD contour |
| `V8TR_LIVE_CLI_BACKEND` | `live-cli-fixture.sh` | Режим сценария `DESIGNER` или `IBCMD`. Обёртки задают режим, для прямого вызова по умолчанию используется `DESIGNER` |
| `V8TR_LIVE_CLI_OUTPUT_ROOT` | `live-cli-designer.sh`, `live-cli-ibcmd.sh`, `live-cli-fixture.sh` | Корень выходных артефактов под `target/manual-tests/*` |
| `V8TR_CI_RUNTIME_ROOT` | `ci-designer-config.sh`, `ci-ibsrv.sh` | Отдельный runtime root для CI infobase, `ibsrv` data/log/pid и generated config |
| `V8TR_INFOBASE_PATH` | `ci-designer-config.sh`, `ci-ibsrv.sh` | Явный file infobase path, синхронизированный между config и `ibsrv --db-path` |
| `V8TR_IBSRV_PATH` | `ci-platform-install.sh`, `ci-ibsrv.sh` | Путь до standalone `ibsrv`, извлечённый из platform bundle |
| `V8TR_DESIGNER_TEST_MODE` | `live-cli-fixture.sh` | Opt-in запуск реального 1С test-stage: `none`, `va`, `yaxunit-all`, `module` |
| `V8TR_DESIGNER_ALLOW_MISSING_CONFIG` | `live-cli-fixture.sh`, `.github/workflows/ci.yml` | Gating hook для soft-skip mandatory live contour на fork/untrusted контексте и на trusted CI без OS-specific platform bundle secrets |
| `V8TR_CI_SKIP_DUPLICATE_RUST_TESTS` | `ci-happy-path.sh`, `.github/workflows/ci.yml` | Явный CI hook для пропуска дублирующего `cargo test` в happy-path, когда contract job уже владеет Rust test coverage |
| `V8TR_REAL_CONFIG` | `live-mcp-http.py` | Реальный config для MCP HTTP smoke |

## Известные нестабильные тесты

Эти тесты падают на `Contract (macos-latest)`, хотя проверяемое изменение их не касается, и
проходят при перезапуске:

- `src/platform/edt_session.rs::cancellation_during_baseline_returns_queued_cancel_without_user_command`
  проверяет состояние в конкретный момент, а не выполнение условия: после отменённой базовой
  линии сессия иногда успевает записать следующую команду. Случаи записаны в #103.

Прежде чем разбирать такое падение, проверьте, меняет ли дифф код этого пути. Не меняет —
перезапустите упавшее задание.

## PR из форка без проверок

PR внешнего участника из форка стоит в `BLOCKED` с пустым списком проверок: CI не
запускался. Так действует политика репозитория `first_time_contributors` (Settings → Actions →
Fork pull request workflows): если у автора PR или у того, чьё действие запустило прогон, нет
в этом репозитории ни влитого коммита, ни влитого PR, GitHub держит прогон в
`action_required` (`gh run list --commit <sha>`) до одобрения мейнтейнера. При более строгой
политике одобрения ждут прогоны любого внешнего участника. Одобряют после чтения диффа:
прогон исполняет чужой код.

## Типовые сценарии запуска

### Contract / Rust CI

```bash
bash scripts/test/ci-rust.sh
```

### Happy-path CI helper

```bash
V8_RUNNER_CI_SCOPE=happy-path bash scripts/test/ci-rust.sh
```

### Trusted CI wiring helpers

`.github/workflows/ci.yml` устанавливает платформу, генерирует конфиг и загружает артефакты при `live_available=true`. Для этого доверенному запуску нужны `V8TR_PLATFORM_BUNDLE_URL_*` и `V8TR_PLATFORM_BUNDLE_SHA256_*` для текущей ОС. Без этих секретов happy-path требует успешных Rust build/check и `live-cli-smoke-test.sh`, а реальные команды 1С пропускает. Полный набор Rust-тестов в этом случае выполняет contract job.

Windows contract scope runs `cli_infobase_cross_platform`, `cli_external_full_pull`, the `support::fs`, declared-infobase reader and infobase-owner unit tests, six Designer CF and DT export argv regressions (verbatim drive paths with Cyrillic and spaces, verbatim UNC paths, and ordinary targets), the `ibcmd config save` argv regression for the same targets, and thirteen named OS regressions (detached and managed-detached stdio EOF, the managed child leading its own console process group, `CTRL_BREAK` cancelling a wait for an external agent, host-owned client termination before handoff and survival after release, directory identity, native config-path resolution, staged-file restore, orphan cleanup, partial-load list sharing, the closed partial-dump list handed to the Designer, the real exit code of an interactive process whose streams closed before its exit was visible); it first verifies that each named test is present in the test list. Its lint step checks the production target only (`cargo clippy --bins`): most integration tests there sit under `#![cfg(unix)]`. Full `cargo test --locked` remains Linux- and macOS-owned until the Windows-specific test suite is hardened. Owner of this contract: this file. Exit criterion: switch Windows contract back to `cargo test --locked` after the path separator, fake-binary, ACL, and process-lifecycle failure groups pass on `windows-latest`.

```bash
bash scripts/test/ci-platform-install.sh
bash scripts/test/ci-designer-config.sh
V8_RUNNER_CI_SCOPE=happy-path bash scripts/test/ci-rust.sh
```

The named Windows regressions also exercise staged `pull` with native Designer and
`ibcmd` fixtures. They cover replacement, refusal, preservation of the previous version
file after an overlay failure, and mode selection without an installed 1C platform.

### Ручной live smoke для Designer

```bash
bash scripts/test/live-cli-designer.sh
```

### Ручной live smoke для IBCMD

```bash
bash scripts/test/live-cli-ibcmd.sh
```

### IBCMD UAT "с нуля"

```bash
bash scripts/test/uat-cli-ibcmd.sh
```

### MCP HTTP smoke

```bash
python3 scripts/test/live-mcp-http.py
```

## Нота по legacy entrypoint

Legacy-скрипт `live-cli.sh` удалён. Вместо него нужно использовать специализированные entrypoint'ы:

- `live-cli-designer.sh` для сценария Конфигуратора
- `live-cli-ibcmd.sh` для сценария IBCMD
