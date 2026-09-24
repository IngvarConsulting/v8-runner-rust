---
id: INV.MCP.AN-UNADMITTED-CALL-IS-A-PROTOCOL-ERROR
check:
  - src/mcp/server.rs::queued_cancellation_returns_transport_error_without_running_call
  - src/mcp/server.rs::an_admission_timeout_returns_a_transport_error
---

# Вызов, не допущенный к исполнению, — ошибка протокола

Вызов, отменённый в очереди или не дождавшийся слота исполнения, получает ошибку протокола
с причиной (`cancelled` или `timeout`) и стадией `queued`, а не ответ инструмента с
`isError`, и сценарий не запускается.
