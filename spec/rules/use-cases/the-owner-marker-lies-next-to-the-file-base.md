---
id: INV.USE-CASES.THE-OWNER-MARKER-LIES-NEXT-TO-THE-FILE-BASE
check:
  - tests/cli_infobase_owner.rs::a_base_without_a_marker_is_taken_and_the_answer_says_so
  - tests/cli_infobase_owner.rs::a_project_copied_whole_leaves_no_live_owner
---

# Метка владельца лежит рядом с файловой базой

Метка владельца лежит рядом с каталогом файловой базы, снаружи него: копия каталога базы
метку не уносит.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
