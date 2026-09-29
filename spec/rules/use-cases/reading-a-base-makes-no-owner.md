---
id: INV.USE-CASES.READING-A-BASE-MAKES-NO-OWNER
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/327
---

# Чтение базы владельцем не делает

Команда чтения на базе другой рабочей копии проходит и в метку ничего не пишет. В словаре
сайта это `infobase dump`, `download`, `extensions list`, `diff --against` и `status --deep`,
а кроме того источник `infobase create --from`. Команды записи и чтения определены в
`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
