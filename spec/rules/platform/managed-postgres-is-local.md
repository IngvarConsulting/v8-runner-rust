---
id: INV.PLATFORM.MANAGED-POSTGRES-IS-LOCAL
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/536
---

# Управляемый PostgreSQL доступен только с этой машины

`dbms start` поднимает PostgreSQL на `127.0.0.1` со входом `scram-sha-256` и сгенерированным паролем.
