---
id: INV.CONFIG.TOP-LEVEL-CONNECTION-IS-NOT-ACCEPTED
check:
  - tests/cli_bootstrap.rs::legacy_top_level_connection_is_rejected_in_json_mode
  - tests/cli_bootstrap.rs::legacy_top_level_credentials_is_rejected_in_json_mode
---

# Строка подключения и учётные данные вне секции базы отклоняются

Ключи подключения и учётных данных на верхнем уровне конфига не принимаются: единственное их место — секция информационной базы.
