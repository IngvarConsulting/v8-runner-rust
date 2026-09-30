---
id: INV.CLI.WITHOUT-ITS-LOCK-A-WRITE-IS-REFUSED
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/326
---

# Без замка базы команда записи отказывает

Если замок файловой базы нельзя взять не потому, что его держит другая команда, команда
записи отказывает и называет каталог и причину, а команда чтения идёт дальше и говорит об
этом. Команды записи и чтения определены в
`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
