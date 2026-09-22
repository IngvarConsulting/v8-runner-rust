---
id: INV.CLI.A-NON-THIN-MODE-STILL-REFUSES-A-STANDALONE-TARGET
status: active
governs: product
decision: DEC.2026-09-16.A-THIN-CLIENT-OPENS-EITHER-ADDRESS
check:
  - tests/cli_agent_standalone.rs::a_non_thin_mode_against_a_standalone_server_is_still_refused
  - tests/cli_agent_standalone.rs::a_standalone_target_refusal_names_the_next_step_as_a_field
scope: [cli]
---

# Второй путь открыт только тонкому клиенту

Конфигуратор, толстый клиент, обычное приложение и `launch mcp --mode thick` против
автономной цели отказывают как прежде, типизированно и с прежним текстом: род
`capability`, код `target`, следующий шаг назван полем `next`. Клиентский адрес —
ws-соединение, и по нему ходит только тонкий клиент.
