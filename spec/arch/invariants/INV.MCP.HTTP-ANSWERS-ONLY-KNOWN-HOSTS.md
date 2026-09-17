---
id: INV.MCP.HTTP-ANSWERS-ONLY-KNOWN-HOSTS
status: active
governs: product
decision: DEC.2026-09-17.THE-HTTP-LISTENER-ANSWERS-ONLY-KNOWN-HOSTS
check: tests/mcp_http.rs::mcp_http_answers_only_the_hosts_it_was_told
scope: [mcp, wire]
---

# Запрос с чужим именем хоста получает отказ, а не ответ

HTTP-слушатель MCP отвечает, только если `Host` называет петлю или имя из
`mcp.http.allowed_hosts`; `Origin` при наличии проверяется тем же списком.
Остальное получает `403` — на любом пути, включая тот, которого нет.
