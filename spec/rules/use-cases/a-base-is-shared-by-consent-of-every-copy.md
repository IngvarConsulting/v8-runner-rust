---
id: INV.USE-CASES.A-BASE-IS-SHARED-BY-CONSENT-OF-EVERY-COPY
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/328
---

# Общей базу делает согласие каждой копии

Если у файловой базы `shared: true` стоит в местном слое каждой держащей её рабочей копии и
этой, проверка владельца пропускает команду записи, и копия записывается в метку рядом с
остальными. Команды записи и чтения определены в
`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
