---
id: INV.USE-CASES.A-MANAGED-TARGET-MEMORY-FOLLOWS-ITS-NAME
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/536
---

# Память управляемой цели привязана к её имени

Адрес управляемой цели в памяти раннера — имя объявленной базы, а не `Srvr=`. Смена портов и перенос кластера на
новый `regport` память не сбрасывают: данные и поколение конфигурации остаются прежними.
