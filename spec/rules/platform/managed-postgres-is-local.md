---
id: INV.PLATFORM.MANAGED-POSTGRES-IS-LOCAL
check:
  - src/platform/postgres.rs::the_address_uses_the_platform_form_and_the_settings_stay_on_the_loopback
  - src/platform/postgres.rs::initdb_takes_the_password_from_a_file_and_never_as_an_argument_value
---

# Управляемый PostgreSQL доступен только с этой машины

`dbms start` поднимает PostgreSQL на `127.0.0.1` со входом `scram-sha-256` и сгенерированным паролем.
