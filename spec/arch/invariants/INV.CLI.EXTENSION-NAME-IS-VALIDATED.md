---
id: INV.CLI.EXTENSION-NAME-IS-VALIDATED
status: active
governs: product
decision: DEC.2026-09-11.BASE-EXTENSIONS-ARE-A-SEPARATE-SUBJECT
check: tests/cli_extensions.rs::extensions_info_rejects_a_name_that_is_not_an_identifier_before_touching_the_infobase
scope: [cli]
---

# Имя расширения проверяется до запуска

Пустое имя и имя, не являющееся идентификатором, отклоняются раньше, чем начнётся работа с базой.
