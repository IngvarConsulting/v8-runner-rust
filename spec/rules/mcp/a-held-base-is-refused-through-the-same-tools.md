---
id: INV.MCP.A-HELD-BASE-IS-REFUSED-THROUGH-THE-SAME-TOOLS
check:
  - tests/cli_infobase_owner.rs::an_mcp_tool_on_a_base_of_another_copy_is_refused_like_the_cli
  - tests/architecture_guardrails.rs::mcp_surface_snapshot_stays_explicit_and_documented
---

# Инструмент MCP на чужой базе отказывает, как командная строка

Инструмент MCP, который на базе другой рабочей копии выполнял бы команду записи, отказывает
так же, как команда командной строки. Команда записи определена в
`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
