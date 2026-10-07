---
id: INV.CLI.A-THICK-CLIENT-REFUSES-A-STANDALONE-TARGET
check:
  - tests/cli_agent_standalone.rs::a_thick_client_against_a_standalone_server_is_refused
  - tests/cli_agent_standalone.rs::a_standalone_target_refusal_names_the_next_step_as_a_field
  - tests/cli_agent_standalone.rs::a_thick_test_client_against_a_standalone_server_is_refused_before_the_build
  - tests/cli_agent_standalone.rs::the_designer_without_the_direct_gate_names_the_connection_string
  - tests/cli_launch.rs::a_standalone_designer_goes_by_the_direct_gate
---

# Толстый клиент и обычное приложение автономную цель не открывают

Против автономной цели `launch thick`, `launch ordinary`, `launch mcp --mode thick|ordinary`
и `test --client-mode thick|ordinary` отказывают типизированно: род `capability`, код
`target`. У `launch` следующий шаг назван полем `next` — тонкий клиент того же вида
запуска: `launch thin` у `launch`, `launch mcp` у `launch mcp`. У `test` следующего шага
нет: поля `next` в отказе нет. Клиент тестов отказывает до сборки, и исходники в базу не
уходят.

`launch designer` против автономной цели не отказывает: Конфигуратор идёт по строке прямого
шлюза. Без строки он отказывает ошибкой валидации, которая называет строку прямого шлюза.
