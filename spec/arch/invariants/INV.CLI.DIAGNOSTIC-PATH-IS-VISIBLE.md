---
id: INV.CLI.DIAGNOSTIC-PATH-IS-VISIBLE
status: active
governs: product
decision: DEC.2026-04-20.ONE-OUTPUT-CONTRACT-FOR-HUMAN-AND-AGENT
check: tests/cli_syntax.rs::syntax_text_success_warning_includes_diagnostic_path
scope: [cli]
---

# Предупреждение называет путь к диагностике

Успех с предупреждением печатает путь, по которому лежит подробность: вызывающему не нужно угадывать, где смотреть.
