---
id: INV.CLI.A-FILE-BASE-IS-LOCKED-FOR-THE-WHOLE-COMMAND
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/326
---

# Файловая база занята на всё время команды

Команда, которая открывает файловую базу, держит замок рядом с её каталогом всё своё время
и этого замка не ждёт: занятая база — отказ, который называет, кто с ней сейчас работает.
Замок действует на одной машине.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
