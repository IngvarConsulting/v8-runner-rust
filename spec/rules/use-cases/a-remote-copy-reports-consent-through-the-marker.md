---
id: INV.USE-CASES.A-REMOTE-COPY-REPORTS-CONSENT-THROUGH-THE-MARKER
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/328
---

# Копия с другой машины сообщает согласие меткой

Рабочая копия с другой машины сообщает согласие меткой: уже записанная в ней копия обновляет
его при каждой своей команде записи, взявшей замок базы, в том числе при отказе по
владельцу. Отзыв виден после её следующей такой команды. Команды записи и чтения определены в
`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
