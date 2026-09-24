---
id: INV.MCP.OVERLOAD-ANSWERS-503-AND-STATELESS-POST-400
check: [tests/mcp_http.rs::mcp_http_max_sessions_returns_503_and_non_initialize_stays_400]
---

# Перегрузка и запрос без сессии отвечают разными кодами

Исчерпание ёмкости сессий отвечает `503`, а обращение без идентификатора сессии там, где он обязателен, — `400`.
