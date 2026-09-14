# ADR-0010: Единый CLI output для человека и AI-агента

- Статус: `accepted`
- Дата: `2026-04-20`

## Контекст

CLI `v8-runner` будет потребляться двумя разными ролями:

1. Человек-разработчик запускает команды вручную и должен быстро увидеть значимые места: что изменилось, что пропущено, что требует внимания, где ошибка и какой следующий шаг.
2. AI-агент запускает CLI как инструмент и тоже нуждается в кратком, предсказуемом, high-signal выводе без лишнего шума.

В предыдущей формулировке решения возникла двусмысленность в двух местах:

1. нужно ли вводить отдельную public ось `audience/profile`, или достаточно одной общей output policy;
2. может ли имя `--output` одновременно означать и формат CLI output, и пользовательский путь результата команды.

Проекту нужен единый формат поведения CLI для обеих ролей, а не две разные presentation-модели.
`json` уже является structured contract для автоматизации и не должен меняться только ради различения "человек vs агент".
Одновременно user-facing path flags должны иметь предсказуемое имя, чтобы `config init`, `launch`, `make/artifacts` и будущий explicit-output `convert` не расходились по `--file` / `--out` / `--output-dir`.

## Решение

Зафиксировать единый high-signal CLI output contract для человека и AI-агента.

1. CLI использует единый high-signal output contract для человека и AI-агента.
2. Единственный публичный selector structured CLI output — булевый флаг `--json-message`.
3. При отсутствии `--json-message` CLI печатает human-readable `text` output.
4. User-facing флаг пути результата должен называться `--output`, если команда публикует один основной output path; отдельные имена `--file`, `--out` и будущий `--output-dir` для того же смысла не вводятся без нового решения.
5. Отдельный public параметр `--audience`, `--profile` или эквивалентный role switch для CLI output не вводится.
6. Оба output mode следуют одной и той же информационной политике:
- clean success остаётся кратким и не печатает подробный успешный журнал без необходимости;
- ошибки, warnings, degraded/skipped behavior, артефакты и diagnostic paths должны быть видимы;
- output должен давать минимальный достаточный сигнал для следующего действия;
- нельзя прятать значимые факты в одном формате и терять их в другом.
7. Machine-readable command payload имеет общий envelope contract для CLI `--json-message` и MCP `structured_content`: `ok`, `command`, `duration_ms`, `data`, `warnings`, `steps` и optional `error` для business failures.
8. CLI `--json-message` продолжает печатать этот envelope как финальный JSON stdout; MCP публикует тот же envelope как command payload внутри native `CallToolResult`/`isError` wrapping.
9. `command` в общем envelope использует canonical CLI command identity (`build`, `test`, `dump`, `syntax`, `launch`, ...). MCP tool identity или scope, если нужен клиенту, сохраняется внутри `data`, а не подменяет `command`.
10. Text output остаётся human-readable mode, но его текст не должен становиться единственным источником важной машинно значимой информации.
11. Use case слой не знает о presentation rules; правила рендеринга остаются в CLI/output и MCP adapters.
12. Любой новый CLI output или новый public path flag должен проверяться на три вопроса:
- помогает ли он следующему действию;
- не добавляет ли шум в clean success path;
- сохраняет ли он ошибки, warnings, degraded behavior, artifacts и diagnostics видимыми в `text` и `json`.

## Неграницы (Non-goals)

1. Немедленное изменение всех текущих CLI сообщений в рамках этого ADR.
2. Замена MCP на CLI для агентской автоматизации.
3. Введение отдельного audience-переключателя.
4. Изменение JSON schema только ради разделения ролей.
5. Скрытие ошибок, предупреждений, degraded behavior или путей к диагностике ради краткости.

## Последствия

1. ADR, backlog и пользовательская документация не должны больше описывать CLI output как две public оси.
2. `--json-message` становится единственным публичным selector-ом structured CLI output.
3. `--output` резервируется для user-facing output path flags.
4. Clean success path должен оставаться кратким в обоих output mode.
5. JSON/envelope остаётся стабильным structured contract для автоматизации и не меняется только из-за различения ролей.
6. MCP `structured_content` использует тот же envelope core fields, при этом protocol-level errors and cancellation/timeout admission failures остаются MCP-native errors.
7. Rendering, help/parse и MCP parity tests должны проверять единый high-signal contract, naming policy and command identity.

## План реализации

1. Зафиксировать этот ADR в `docs/decisions`.
2. Добавить invariant в `spec/architecture/invariants.md`.
3. Синхронизировать `ADR-0010`, `spec/architecture/invariants.md`, arc42 и backlog с единой моделью output policy и naming policy для output paths.
4. При реализации unified output policy обновить:
- `src/output/presenter.rs`
- `src/output/text.rs`
- `src/cli/execute.rs`, если меняется wiring/selection rendering rules
- `src/cli/args.rs`
- `src/app.rs`
- `docs/CAPABILITIES.md`
- `README.md`
- `docs/CONFIGURATION.md`
5. Shared envelope contract lives outside `src/output`, `src/cli`, `src/mcp`, and `src/use_cases`; adapters may import it, but use cases must not.
6. Ввести и поддерживать rendering tests для единого output contract:
- clean success не шумит;
- ошибки/warnings/degraded/artifacts/actionable diagnostics видимы;
- `text` и `json` не противоречат друг другу по ключевым фактам.
- CLI `--json-message` и MCP `structured_content` имеют одинаковый envelope core для `build`, `test`, `dump`, `syntax` и business failures.
7. Ввести и поддерживать help/parse regressions для public contract:
- `--json-message` выбирает structured output без изменения JSON schema;
- `config init --output`, `launch --output`, `make/artifacts --output` и `convert --output` используют одно и то же user-facing имя;
- `config init` не использует глобальный `--config` как shortcut output path.
8. При добавлении новой CLI команды, нового result field или нового output path flag обновлять rendering/help/parse tests before exposing the behavior.

## Верификация

- [x] ADR фиксирует единый CLI output contract для человека и AI-агента.
- [x] ADR фиксирует, что единственный public selector structured output — `--json-message`.
- [x] ADR явно отвергает отдельный audience/profile-переключатель для CLI output.
- [x] ADR сохраняет текущую роль JSON как structured contract.
- [x] ADR резервирует `--output` для user-facing output path flags.
- [x] ADR сохраняет транспортно-нейтральный use case слой согласно ADR-0006.
- [x] ADR фиксирует общий envelope core для CLI `--json-message` и MCP `structured_content`.
