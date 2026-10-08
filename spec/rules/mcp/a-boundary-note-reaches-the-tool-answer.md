---
id: INV.MCP.A-BOUNDARY-NOTE-REACHES-THE-TOOL-ANSWER
check:
  - tests/cli_infobase_owner.rs::an_mcp_tool_on_a_base_of_another_copy_runs_like_the_cli
  - tests/cli_infobase_owner.rs::an_mcp_tool_answer_names_the_takeover_and_the_gone_owner
  - src/mcp/service.rs::a_boundary_warning_leads_a_refusal_too
---

# Предупреждения границы названы в ответе инструмента MCP

Когда инструмент MCP пишет в базу другой рабочей копии, берёт базу без метки или сменяет
ушедшего владельца (`INV.USE-CASES.A-WRITE-ON-A-BASE-OF-ANOTHER-COPY-RUNS-WITH-A-WARNING`,
`INV.USE-CASES.A-TAKEOVER-IS-NAMED-IN-THE-ANSWER`,
`INV.USE-CASES.A-GONE-OWNER-IS-REPLACED-AND-NAMED`), его ответ говорит об этом так же, как
ответ командной строки: те же тексты идут первыми в `warnings` его ответа — и успеха, и
отказа.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
