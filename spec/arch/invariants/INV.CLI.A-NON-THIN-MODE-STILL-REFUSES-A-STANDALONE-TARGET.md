---
id: INV.CLI.A-NON-THIN-MODE-STILL-REFUSES-A-STANDALONE-TARGET
status: active
governs: product
decision: DEC.2026-09-16.A-THIN-CLIENT-OPENS-EITHER-ADDRESS
check: tests/cli_agent_standalone.rs::a_non_thin_mode_against_a_standalone_server_is_still_refused
scope: [cli]
---

# Второй путь открыт только тонкому клиенту

Конфигуратор, толстый клиент, обычное приложение и `launch mcp --mode thick` против
автономной цели отказывают как прежде, типизированно и с прежним текстом. Клиентский
адрес — ws-соединение, и по нему ходит только тонкий клиент.
