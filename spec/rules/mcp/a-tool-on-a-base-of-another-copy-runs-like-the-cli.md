---
id: INV.MCP.A-TOOL-ON-A-BASE-OF-ANOTHER-COPY-RUNS-LIKE-THE-CLI
check:
  - tests/cli_infobase_owner.rs::an_mcp_tool_on_a_base_of_another_copy_runs_like_the_cli
  - tests/architecture_guardrails.rs::mcp_surface_snapshot_stays_explicit_and_documented
---

# Инструмент MCP на базе другой копии идёт, как командная строка

Инструмент MCP, который на базе другой рабочей копии выполняет команду записи, не
отказывает и метку не меняет — так же, как команда командной строки
(`INV.USE-CASES.A-WRITE-ON-A-BASE-OF-ANOTHER-COPY-RUNS-WITH-A-WARNING`). Попадёт ли
предупреждение в ответ инструмента, говорит `INV.MCP.A-BOUNDARY-NOTE-REACHES-THE-TOOL-ANSWER`.
Команда записи определена в `INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
