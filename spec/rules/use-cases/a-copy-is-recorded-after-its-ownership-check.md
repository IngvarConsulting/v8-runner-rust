---
id: INV.USE-CASES.A-COPY-IS-RECORDED-AFTER-ITS-OWNERSHIP-CHECK
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/327
---

# Копию в метку записывает команда, прошедшая проверку владельца

Свою рабочую копию в метку записывает команда записи на базе, названной в местном слое,
когда проверка владельца пройдена. Отказ по владельцу новую копию в метку не записывает;
превью и невзятый замок метку не меняют. Команды записи и чтения определены в
`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
