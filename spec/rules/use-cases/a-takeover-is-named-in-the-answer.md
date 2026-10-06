---
id: INV.USE-CASES.A-TAKEOVER-IS-NAMED-IN-THE-ANSWER
check:
  - tests/cli_infobase_owner.rs::a_base_without_a_marker_is_taken_and_the_answer_says_so
---

# Взятие базы без метки названо в ответе

Когда рабочая копия первой записывается в метку существующей базы, у которой метки не было,
ответ команды командной строки говорит, что база теперь за ней. Инструменту MCP это
обещает `INV.MCP.A-BOUNDARY-NOTE-REACHES-THE-TOOL-ANSWER`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
