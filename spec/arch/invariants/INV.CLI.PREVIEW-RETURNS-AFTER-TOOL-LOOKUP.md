---
id: INV.CLI.PREVIEW-RETURNS-AFTER-TOOL-LOOKUP
status: active
governs: product
decision: DEC.2026-09-22.PREVIEW-LOGGING-FOLLOWS-THE-COMMAND-CONTRACT
check: tests/contract_previews.rs::a_preview_refuses_before_naming_a_plan_when_the_platform_is_missing
scope: [cli]
---

# Отсутствие платформы отказывает до одобрения плана

Превью возвращается после поиска утилиты: если платформы нет, вызывающий узнаёт это раньше, чем одобрит план.
