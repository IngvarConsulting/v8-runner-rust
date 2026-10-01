---
id: INV.PLATFORM.AGENT-READINESS-IS-AUTHENTICATION
check: [tests/cli_dump_agent.rs::a_rejected_password_is_an_environment_refusal_even_though_the_port_answers]
---

# Готовность агента доказывает аутентификация

Провайдер считается доступным по успешной SSH-аутентификации с настроенными учётными данными или пустой парой, а не по открытому порту.
