---
id: CTR.MCP.PUBLISHED-TOOL-SURFACE
status: active
governs: product
version: 1
decision: DEC.2026-04-20.MCP-DOES-NOT-MIRROR-CLI
producer: src/mcp/service.rs
consumers: [mcp, docs]
check: tests/architecture_guardrails.rs::mcp_surface_snapshot_stays_explicit_and_documented
scope: [mcp, wire]
---

# Состав опубликованных инструментов MCP

Перечень инструментов — наблюдаемая форма: клиент видит именно его. Состав объявлен
здесь, сверяется с кодом и с документацией, а его изменение меняет версию формы.

Опубликованы восемь инструментов:

- `run_all_tests`
- `run_module_tests`
- `build_project`
- `dump_config`
- `launch_app`
- `check_syntax_edt`
- `check_syntax_designer_config`
- `check_syntax_designer_modules`

Состав меняется только вместе с версией этой формы.
