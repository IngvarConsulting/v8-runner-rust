---
id: INV.CLI.EXTENSION-NAME-IS-VALIDATED
check: [tests/cli_extensions.rs::extensions_info_rejects_a_name_that_is_not_an_identifier_before_touching_the_infobase]
---

# Имя расширения проверяется до запуска

Пустое имя и имя, не являющееся идентификатором, отклоняются раньше, чем начнётся работа с базой.
