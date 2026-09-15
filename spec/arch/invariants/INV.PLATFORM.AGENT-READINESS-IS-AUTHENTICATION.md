---
id: INV.PLATFORM.AGENT-READINESS-IS-AUTHENTICATION
status: active
governs: product
decision: DEC.2026-09-14.AGENT-SPEAKS-JSON-WITHOUT-A-PTY
check: tests/cli_dump_agent.rs::a_rejected_password_is_an_environment_refusal_even_though_the_port_answers
scope: [platform]
---

# Готовность агента доказывает аутентификация

Провайдер считается доступным по успешной SSH-аутентификации с настроенными учётными данными или пустой парой, а не по открытому порту.
