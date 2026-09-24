---
id: INV.MCP.SURFACE-STAYS-EXPLICIT
check: [tests/architecture_guardrails.rs::mcp_surface_snapshot_stays_explicit_and_documented]
---

# Состав инструментов MCP перечислен поимённо

Набор опубликованных инструментов задан явным перечнем в `src/mcp/server.rs`, и запись `CTR.MCP.PUBLISHED-TOOL-SURFACE` называет тот же набор теми же именами в том же порядке. Страж сверяет оба перечня с закреплённым у себя набором имён, поэтому расхождение кода и записи видно на любой стороне.
