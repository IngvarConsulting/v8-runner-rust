---
id: INV.USE-CASES.A-CONNECTION-STRING-BASE-OBEYS-OWNERSHIP-WITHOUT-OWNING
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/327
---

# Строка соединения подчиняется владельцу, но им не становится

Команда записи со строкой соединения в `--infobase` на файловой базе другой рабочей копии
отказывает так же, как с именем базы; свою копию в метку она не записывает ни на какой базе,
в том числе без метки или с ушедшим владельцем. Команды записи и чтения определены в
`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
