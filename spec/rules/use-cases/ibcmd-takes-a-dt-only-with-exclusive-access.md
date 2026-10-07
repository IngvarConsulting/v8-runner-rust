---
id: INV.USE-CASES.IBCMD-TAKES-A-DT-ONLY-WITH-EXCLUSIVE-ACCESS
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/435
---

# `ibcmd` снимает и загружает `.dt` только при монопольном доступе

`ibcmd` исполняет `infobase dump` и `infobase restore` файловой базы только по явному
ключу `providers.infobase.dump|restore: ibcmd`, вне цепочки умолчаний, и запускается
лишь после того, как раннер убедился в монопольном доступе к базе: активных соединений
нет либо монопольный доступ получен. При открытом сеансе команда отказывает до запуска
`ibcmd infobase dump` или `ibcmd infobase restore`, база и `.dt` не тронуты; без сеансов
снимок снимается и загружается. Признак сеанса читается структурно, а не из прозы
платформы.

