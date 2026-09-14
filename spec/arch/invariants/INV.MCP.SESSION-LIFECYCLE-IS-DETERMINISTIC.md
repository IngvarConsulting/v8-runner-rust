---
id: INV.MCP.SESSION-LIFECYCLE-IS-DETERMINISTIC
status: active
governs: product
decision: DEC.2026-04-20.MCP-LIMITS-EXECUTION-AND-SESSIONS-SEPARATELY
check: tests/mcp_http.rs::mcp_http_missing_and_expired_sessions_are_deterministic
scope: [mcp, wire]
---

# Отсутствующая и истёкшая сессия отвечают предсказуемо

Обращение без идентификатора сессии и обращение по истёкшей сессии дают заранее определённые ответы, а не случайный сбой.
