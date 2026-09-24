---
id: INV.CLI.DIAGNOSTIC-PATH-IS-VISIBLE
check: [tests/cli_syntax.rs::syntax_with_an_unreadable_log_refuses_instead_of_reporting_clean]
---

# Предупреждение называет путь к диагностике

Успех с предупреждением печатает путь, по которому лежит подробность: вызывающему не нужно угадывать, где смотреть.
