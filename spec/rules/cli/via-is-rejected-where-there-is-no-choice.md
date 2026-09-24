---
id: INV.CLI.VIA-IS-REJECTED-WHERE-THERE-IS-NO-CHOICE
check:
  - tests/cli_launch.rs::via_is_refused_where_there_is_no_choice
  - tests/mcp_stdio.rs::mcp_stdio_launch_app_refuses_via_where_there_is_no_choice
---

# Ключ выбора адреса отвергается там, где выбирать нечего

`--via` у режима с одним адресом — Конфигуратора, толстого клиента, обычного приложения,
`launch web` — типизированный отказ, а не молчаливое принятие: принять ключ и ничего им не
выбрать значит соврать вызывающему. Правило одинаково на обеих поверхностях: у MCP тот же
ключ и тот же отказ.
