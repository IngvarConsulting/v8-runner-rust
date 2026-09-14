# ADR-0007: Свести EDT execution к one-shot и shared interactive режимам

- Статус: `accepted`
- Дата: `2026-04-20`

## Контекст

EDT-сценарии могут выполняться двумя принципиально разными способами:

1. one-shot: каждый вызов запускает отдельный `1cedtcli` с параметрами и ждёт завершения процесса;
2. shared interactive: один живой interactive `1cedtcli` принимает команды через `stdin/stdout`, а доступ к нему управляется shared actor/manager.

Отдельный non-shared interactive режим усложняет архитектуру без самостоятельной пользовательской ценности.
Если interactive EDT создаётся напрямую в каждом CLI/use-case вызове, система получает третий execution path: он уже не one-shot, но ещё не shared session с едиными baseline/restart/drain правилами.

Целевой пользовательский выбор должен оставаться простым: либо запускать `edtcli` каждый раз с параметрами, либо использовать общую interactive EDT session.

## Решение

EDT execution должен иметь ровно два целевых режима.

1. `tools.edt_cli.interactive_mode=false` означает one-shot EDT execution: каждый EDT-вызов запускает отдельный `1cedtcli` с параметрами.
2. `tools.edt_cli.interactive_mode=true` означает shared interactive EDT execution: поддержанные EDT-команды идут через общий EDT actor/manager и общую interactive session.
3. В публичной конфигурации `interactive_mode` не означает отдельный non-shared interactive режим.
4. Shared interactive режим должен иметь единый lifecycle contract: admission/queue, baseline reset/probe перед пользовательской командой, restart semantics, shutdown/restart drain, typed errors and telemetry.
5. `tools.edt_cli.auto_start=true` означает eager prewarm shared interactive EDT session только для long-lived host process, который реально переиспользует один actor/manager между несколькими запросами. На текущем этапе это относится к MCP server.
6. Для long-lived процесса, например MCP server, shared interactive session переиспользуется между запросами.
7. Для short-lived CLI процесса EDT при `interactive_mode=true` стартует лениво при первом EDT-вызове и живёт только в рамках текущего процесса команды. `auto_start` не должен превращаться в eager prewarm каждого CLI invocation.
8. Sharing между независимыми OS process invocations считается преждевременным усложнением и не входит в этот ADR. Если когда-либо появится подтверждённая необходимость, для этого нужен отдельный daemon/server ADR.
9. Если текущая реализация создаёт interactive EDT session напрямую внутри CLI/use case и не использует shared actor/manager, это считается implementation gap, а не целевой архитектурой.
10. Если shared interactive временно покрывает не все EDT-сценарии, gap должен быть явно описан в backlog/docs/ADR, а не считаться архитектурной нормой.

## Неграницы (Non-goals)

1. Обязательное удаление one-shot EDT mode.
2. Обещание shared EDT session между отдельными CLI process invocations без отдельного daemon/server.
3. Немедленная реализация shared interactive для всех EDT-сценариев в рамках этого ADR.
4. Eager prewarm interactive EDT для каждого short-lived CLI процесса.
5. Использование shared interactive EDT как скрытой оптимизации без публичного конфигурационного контракта.
6. Сохранение отдельного non-shared interactive режима как долгосрочной публичной модели.

## Последствия

1. Документация должна описывать только два режима EDT: one-shot и shared interactive.
2. `tools.edt_cli.interactive_mode` является публичным переключателем shared interactive EDT.
3. `tools.edt_cli.auto_start` должен документироваться как eager prewarm shared session только для long-lived host process; на текущем этапе это MCP server.
4. CLI при `interactive_mode=true` стартует EDT лениво при первом EDT-вызове и не использует `auto_start` как startup prewarm для короткоживущего процесса.
5. Прямые per-command interactive sessions в CLI/use cases должны быть устранены или явно помечены как transition gap до устранения.
6. MCP shared actor не должен оставаться MCP-only архитектурной нормой, если CLI и другие EDT-сценарии включают `interactive_mode=true`.
7. Реализация shared interactive должна быть проверяема тестами на доступность операций, timeout behavior, baseline isolation, restart behavior and shutdown/restart drain behavior.
8. Future agents must not introduce a third EDT execution path between one-shot and shared interactive.

## План реализации

1. Сохранить `tools.edt_cli.interactive_mode` как публичный переключатель shared interactive EDT.
2. Синхронизировать конфигурацию и документацию:
- `src/config/model.rs`
- `src/config/validate.rs`
- `docs/CONFIGURATION.md`
- `docs/CAPABILITIES.md`
- `ARCHITECTURE.md`
3. Устранить прямое создание non-shared interactive EDT sessions в use cases:
- `src/use_cases/init_project.rs`
- `src/use_cases/build_project.rs`
- `src/use_cases/check_syntax.rs`
4. Вынести shared EDT actor/manager из MCP-only boundary в `src/platform` или общий execution слой:
- MCP остаётся controller/DTO/presenter boundary и не содержит собственной EDT execution-логики;
- CLI и MCP используют один shared EDT component;
- текущий `src/mcp/edt_session.rs` должен быть перенесён или превращён в thin adapter над общим component;
- затронутые области: `src/platform/edt.rs`, `src/platform/interactive.rs`, общий execution слой, `src/mcp/edt_session.rs`.
5. Добавить tests:
- `interactive_mode=false` uses one-shot `1cedtcli` execution;
- `interactive_mode=true` routes supported EDT commands through shared actor/manager;
- direct non-shared interactive session не используется как публичный execution path;
- shared actor сохраняет baseline/restart/drain contract.
6. Backlog gap по текущему расхождению фиксируется в `spec/ADR_DERIVED_BACKLOG.md` как `ADR-TASK-004`.

## Верификация

- [x] ADR фиксирует два целевых режима EDT: one-shot и shared interactive.
- [x] ADR фиксирует `tools.edt_cli.interactive_mode` как переключатель shared interactive EDT.
- [x] ADR запрещает сохранять non-shared interactive как долгосрочный публичный режим.
- [x] ADR требует описывать неполное покрытие shared EDT как gap.
- [x] Инвариант добавлен в архитектурную документацию.
