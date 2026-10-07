---
id: INV.USE-CASES.IBCMD-RS-FOLLOWS-IBCMD-IN-THE-CONVERT-CHAIN
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/413
---

# `ibcmd-rs` идёт за `ibcmd` в цепочке `convert`

В строке `convert` за `ibcmd` стоит `ibcmd-rs`: он переводит XML в пакет и пакет в XML без
базы и без платформы, и без `ibcmd` направление с пакетом исполняет он. Пока его вызов,
входы и выходы, поддержка `.cfe` и работа в Windows не замерены, адаптера у него нет.
