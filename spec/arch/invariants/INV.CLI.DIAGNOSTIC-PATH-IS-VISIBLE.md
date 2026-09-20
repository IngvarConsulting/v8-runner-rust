---
id: INV.CLI.DIAGNOSTIC-PATH-IS-VISIBLE
status: active
governs: product
decision: DEC.2026-04-20.ONE-OUTPUT-CONTRACT-FOR-HUMAN-AND-AGENT
check: tests/cli_syntax.rs::syntax_with_an_unreadable_log_refuses_instead_of_reporting_clean
scope: [cli]
---

# Предупреждение называет путь к диагностике

Успех с предупреждением печатает путь, по которому лежит подробность: вызывающему не нужно угадывать, где смотреть.
