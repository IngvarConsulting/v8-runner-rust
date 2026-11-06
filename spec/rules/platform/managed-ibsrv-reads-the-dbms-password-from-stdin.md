---
id: INV.PLATFORM.MANAGED-IBSRV-READS-THE-DBMS-PASSWORD-FROM-STDIN
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/537
---

# Пароль СУБД уходит в `ibsrv` и `ibcmd` через stdin

Для базы в СУБД `ibsrv` и `ibcmd infobase create` получают пароль ключом `-W` через stdin, а в командной строке его нет.
