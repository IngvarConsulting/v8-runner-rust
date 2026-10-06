---
id: INV.MCP.A-BOUNDARY-NOTE-REACHES-THE-TOOL-ANSWER
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/404
---

# Взятие базы и смена владельца названы в ответе инструмента MCP

Когда инструмент MCP берёт базу без метки или сменяет ушедшего владельца
(`INV.USE-CASES.A-TAKEOVER-IS-NAMED-IN-THE-ANSWER`,
`INV.USE-CASES.A-GONE-OWNER-IS-REPLACED-AND-NAMED`), его ответ говорит об этом так же, как
ответ командной строки, а не только журнал сервера.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
