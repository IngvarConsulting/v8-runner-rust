---
id: INV.CONFIG.TOP-LEVEL-CONNECTION-IS-NOT-ACCEPTED
status: active
governs: product
decision: DEC.2026-04-21.INFOBASE-SECTION-OWNS-CONNECTION-AND-CREDENTIALS
check: [tests/cli_bootstrap.rs::legacy_top_level_connection_is_rejected_in_json_mode, tests/cli_bootstrap.rs::legacy_top_level_credentials_is_rejected_in_json_mode]
scope: [config]
---

# Строка подключения и учётные данные вне секции базы отклоняются

Ключи подключения и учётных данных на верхнем уровне конфига не принимаются: единственное их место — секция информационной базы.
