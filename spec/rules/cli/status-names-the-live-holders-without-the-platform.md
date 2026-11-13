---
id: INV.CLI.STATUS-NAMES-THE-LIVE-HOLDERS-WITHOUT-THE-PLATFORM
check:
  - tests/cli_infobase_owner.rs::status_without_deep_names_the_live_holders_without_the_platform
---

# `status` называет живых владельцев базы без платформы

У файловой базы `status` — с `--deep` и без него, и у `--all` для каждой файловой базы —
называет в `holders` копии из метки владельца, которые держат базу: эту и другие живые.
Копию, которую команда записи сменила бы как ушедшую
(`INV.USE-CASES.A-GONE-OWNER-IS-REPLACED-AND-NAMED`), он живой не числит, а метку не
меняет. Платформа не запускается
(`INV.CLI.STATUS-WITHOUT-DEEP-STARTS-NO-PLATFORM`), в метку ничего не пишется
(`INV.USE-CASES.READING-A-BASE-MAKES-NO-OWNER`). Так вызывающий может спросить человека о
чужой базе до записи, не делая сухого прогона каждой команды.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
