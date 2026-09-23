---
id: INV.CLI.PREVIEW-RETURNS-AFTER-TOOL-LOOKUP
check:
  - tests/contract_previews.rs::a_preview_refuses_before_naming_a_plan_when_the_platform_is_missing
  - tests/cli_build.rs::a_planned_edt_build_refuses_when_the_utility_that_would_load_it_is_missing
---

# Отсутствие платформы отказывает до одобрения плана

Превью возвращается после поиска утилиты: если платформы нет, вызывающий узнаёт это
раньше, чем одобрит план. Искать превью обязано то же, что запустил бы боевой прогон, —
иначе оно отказывает там, где применение работает.
