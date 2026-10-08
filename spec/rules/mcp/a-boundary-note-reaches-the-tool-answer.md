---
id: INV.MCP.A-BOUNDARY-NOTE-REACHES-THE-TOOL-ANSWER
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/404
---

# Предупреждения границы названы в ответе инструмента MCP

Когда инструмент MCP пишет в базу другой рабочей копии, берёт базу без метки или сменяет
ушедшего владельца (`INV.USE-CASES.A-WRITE-ON-A-BASE-OF-ANOTHER-COPY-RUNS-WITH-A-WARNING`,
`INV.USE-CASES.A-TAKEOVER-IS-NAMED-IN-THE-ANSWER`,
`INV.USE-CASES.A-GONE-OWNER-IS-REPLACED-AND-NAMED`), его ответ говорит об этом так же, как
ответ командной строки, а не только журнал сервера.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
