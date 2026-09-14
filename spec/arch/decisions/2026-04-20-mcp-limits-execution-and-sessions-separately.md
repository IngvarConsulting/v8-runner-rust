---
id: DEC.2026-04-20.MCP-LIMITS-EXECUTION-AND-SESSIONS-SEPARATELY
status: active
governs: product
realized: tests/mcp_http.rs::mcp_http_missing_and_expired_sessions_are_deterministic
supersedes: []
superseded-by: null
establishes: [INV.MCP.ADMISSION-IS-SHARED-BY-BOTH-TRANSPORTS, INV.MCP.OVERLOAD-ANSWERS-503-AND-STATELESS-POST-400, INV.MCP.SESSION-LIFECYCLE-IS-DETERMINISTIC]
---

# Нагрузка и ёмкость сессий ограничиваются порознь

**Решение.** У MCP два независимых ограничителя. Первый допускает к исполнению не
больше заданного числа одновременных вызовов инструментов — одинаково для обоих
транспортов, различая стадии ожидания и исполнения. Второй ограничивает число
отслеживаемых сессий HTTP через резервирование, подтверждение и освобождение, с
вытеснением по простою. Ни один не заменяет блокировку рабочего каталога.

**Почему.** Это разные ресурсы: сессия занимает память сервера, даже когда ничего
не исполняет, а исполнение занимает платформу, даже если сессия одна.

**Не затрагивает.** Семантику отмены: она приходит из общей политики исполнения.
