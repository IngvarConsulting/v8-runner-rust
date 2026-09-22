---
id: INV.CLI.PREVIEW-RETURNS-AFTER-TOOL-LOOKUP
status: active
governs: product
decision: DEC.2026-09-11.PREVIEW-STOPS-BEFORE-THE-PROVIDER-IS-DISPATCHED
check:
  - tests/contract_previews.rs::a_preview_refuses_before_naming_a_plan_when_the_platform_is_missing
  - tests/cli_build.rs::a_planned_edt_build_refuses_when_the_utility_that_would_load_it_is_missing
scope: [cli]
---

# Отсутствие платформы отказывает до одобрения плана

Превью возвращается после поиска утилиты: если платформы нет, вызывающий узнаёт это раньше, чем одобрит план.
