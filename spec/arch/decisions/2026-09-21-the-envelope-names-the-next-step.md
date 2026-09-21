---
id: DEC.2026-09-21.THE-ENVELOPE-NAMES-THE-NEXT-STEP
status: planned
governs: product
realized: null
supersedes: []
superseded-by: null
establishes: []
changes: [CTR.WIRE.COMMAND-ENVELOPE]
---

# Отказ называет следующий шаг отдельным полем

**Решение.** Машинный отказ несёт `error.next` — следующий шаг структурой: `command`,
при нужде `source_set` и ключи, — а не подсказкой в тексте. Отказ `non_fast_forward`
целиком: `kind`, `code`, `message`, `base_generation`, `local_generation`, `next`;
пример: `{"ok": false, "error": {"kind": "non_fast_forward", "base_generation": "…",
"local_generation": "…", "next": {"command": "pull", "source_set": "main"}}}`. Состав
полей `error` остаётся закрытым и растёт вместе с версией конверта. Текстовая строка
остаётся человеку.

**Почему.** Сценарий сборки и Unica читают отказ машиной: следующий шаг, спрятанный в
прозе, приходится разбирать регулярным выражением.

**Не затрагивает.** Поверхность MCP `CTR.MCP.PUBLISHED-TOOL-SURFACE` — имена и схемы
входа; конверт в ответах инструментов MCP меняется вместе с командной строкой, `data`
отказа (`CTR.WIRE.REFUSAL-DATA`) — нет.

Источник: [`cli.html#refusals`](../../../docs/site/cli.html#refusals).
