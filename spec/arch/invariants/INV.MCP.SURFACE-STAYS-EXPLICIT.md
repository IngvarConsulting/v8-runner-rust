---
id: INV.MCP.SURFACE-STAYS-EXPLICIT
status: active
governs: product
decision: DEC.2026-04-20.MCP-DOES-NOT-MIRROR-CLI
check: tests/architecture_guardrails.rs::mcp_surface_snapshot_stays_explicit_and_documented
scope: [mcp]
---

# Состав инструментов MCP назван явно и всюду одинаково

Перечень инструментов в `src/mcp/server.rs`, перечень в тексте
`CTR.MCP.PUBLISHED-TOOL-SURFACE` и закреплённый снимок проверки совпадают. Инструмент,
заведённый в коде, валит проверку, пока его не назовут обе остальные стороны.
