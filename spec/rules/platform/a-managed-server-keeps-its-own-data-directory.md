---
id: INV.PLATFORM.A-MANAGED-SERVER-KEEPS-ITS-OWN-DATA-DIRECTORY
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/537
---

# У поднятого раннером `ibsrv` свой каталог `--data`

Каталог `--data` лежит под `managed/` и не совпадает с `ibcmd-data`, который раннер отдаёт разовым запускам `ibcmd`.
