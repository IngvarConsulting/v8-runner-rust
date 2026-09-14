---
id: INV.CLI.SECRETS-NEVER-REACH-THE-OUTPUT
status: active
governs: product
decision: DEC.2026-09-11.SECRETS-ARE-MASKED-IN-EVERY-OUTPUT
check: [tests/cli_launch.rs::launch_dry_run_text_masks_credentials_and_says_nothing_was_dispatched, tests/cli_extensions.rs::extension_preview_never_echoes_the_infobase_password]
scope: [cli]
---

# Пароль не появляется в выводе

Ни превью, ни квитанция не печатают пароль информационной базы — ни отдельным значением, ни внутри составленной строки запуска.
