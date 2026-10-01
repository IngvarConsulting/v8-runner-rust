---
id: INV.MCP.SESSION-LIFECYCLE-IS-DETERMINISTIC
check: [tests/mcp_http.rs::mcp_http_missing_and_expired_sessions_are_deterministic]
---

# Отсутствующая и истёкшая сессия отвечают предсказуемо

Обращение без идентификатора сессии и обращение по истёкшей сессии дают заранее определённые ответы, а не случайный сбой.
