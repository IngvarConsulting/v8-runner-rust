# ADR-0005: Разделить CLI и MCP публичные поверхности

- Статус: `accepted`
- Дата: `2026-04-20`

## Контекст

`v8-runner` имеет две публичные поверхности: CLI и MCP.
CLI содержит полный набор пользовательских команд, включая `config init`, `init`, `extensions`, `build`, `load`, `test`, `dump`, `convert`, `make`/`artifacts`, `syntax`, `launch` и `mcp serve`.
MCP предназначен для управляемого агентского доступа и сейчас публикует более узкий набор tool-операций.

Если считать MCP автоматическим зеркалом CLI, любое CLI-расширение будет неявно становиться MCP-контрактом.
Это повышает риск небезопасного роста поверхности, размывает ожидания MCP-клиентов и усложняет версионирование.

## Решение

Разделить CLI и MCP как самостоятельные публичные поверхности.

1. MCP не зеркалит CLI автоматически.
2. Текущая MCP-поверхность состоит ровно из 8 опубликованных tool-операций:
- `run_all_tests`
- `run_module_tests`
- `build_project`
- `dump_config`
- `launch_app`
- `check_syntax_edt`
- `check_syntax_designer_config`
- `check_syntax_designer_modules`
3. CLI может иметь команды, не опубликованные в MCP. `convert` является явным примером такого CLI-only сценария.
4. Добавление новой MCP tool-операции, удаление существующей tool-операции, переименование или изменение ее семантики считается изменением MCP public surface.
5. Расширение MCP surface требует отдельного ADR или явного обновления этого ADR с описанием мотивации, DTO, бизнес-ошибок, ограничений исполнения и тестов.
6. MCP request DTO используют собственную форму и не обязаны совпадать с CLI flags.
   Для `launch_app` допустимое расширение в рамках текущего набора tools — явные поля client MCP:
   `utilityType=mcp`, `mcpScenario`, `mode`, `mcpConfig`, `mcpPort`, `waitReady`.
   Для `run_all_tests` допустимое расширение — выбор runner: `runner=yaxunit|vanessa` и Vanessa-поля
   `profile`, `feature`, `filterTag`, `ignoreTag`, `scenarioFilter`.
7. MCP command response payload в `structured_content` использует общий machine-readable command envelope с CLI `--json-message`: `ok`, `command`, `duration_ms`, `data`, `warnings`, `steps` и optional `error` для business failures. MCP protocol wrapping (`CallToolResult`, `isError`, transport/internal `ErrorData`) остается MCP-native.
8. Внутри `data` MCP может временно сохранять command-specific compatibility fields или tool/scope identity, но это явная compatibility boundary, а не новый use-case/domain contract.

## Неграницы (Non-goals)

1. Немедленная публикация всех CLI-команд в MCP.
2. Создание стабильного ABI между CLI flags и MCP request DTO.
3. Запрет на будущие MCP tool-операции; запрещено только неявное расширение без решения.

## Последствия

1. Документация должна явно перечислять опубликованные MCP tools.
2. Реализация MCP должна оставаться в `src/mcp`, а не проходить через `cli::execute`.
3. Новые CLI-команды не требуют MCP-эквивалента по умолчанию.
4. Новые MCP tools должны проектироваться как отдельный контракт: request DTO, command envelope data payload, business failure shape, runtime/admission behavior, telemetry and tests.
5. Агентам нельзя выводить доступность MCP-операции из наличия CLI-команды.
6. Изменение MCP `structured_content` envelope или compatibility data считается изменением MCP public surface и требует ADR/docs/test sync.

## План реализации

1. Зафиксировать этот ADR в `docs/decisions`.
2. Обновить индекс ADR и arc42 decision list.
3. Зафиксировать invariant в `spec/architecture/invariants.md`.
4. При изменении MCP surface синхронизировать:
- `README.md`
- `docs/CAPABILITIES.md`
- `ARCHITECTURE.md`
- `src/mcp/server.rs`
- `src/mcp/request.rs`
- `src/mcp/service.rs`
- `src/command_envelope.rs`, если меняется shared command payload
5. Добавлять/обновлять тесты MCP server/service для каждого изменения surface.

## Верификация

- [x] ADR явно говорит, что MCP не зеркалит CLI.
- [x] ADR перечисляет 8 текущих MCP tool-операций.
- [x] ADR требует отдельного решения для расширения MCP surface.
- [x] Инвариант добавлен в архитектурную документацию.
