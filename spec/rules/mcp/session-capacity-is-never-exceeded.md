---
id: INV.MCP.SESSION-CAPACITY-IS-NEVER-EXCEEDED
check:
  - tests/mcp_http.rs::mcp_http_initialize_burst_respects_capacity_and_recovers_after_delete
  - tests/mcp_http.rs::mcp_http_parallel_initialize_respects_max_sessions
  - src/mcp/server.rs::http_session_reservation_drop_releases_capacity
---

# Ёмкость сессий HTTP не превышается и при одновременных запросах

Одновременные `initialize` не открывают сессий сверх `mcp.http.max_sessions`. Сессия,
закрытая `DELETE`, освобождает место для новой, а неудавшийся `initialize` места не
занимает.
