---
id: INV.USE-CASES.THE-OWNER-MARKER-RECORDS-EACH-OWNING-COPY
check:
  - tests/cli_infobase_owner.rs::a_base_without_a_marker_is_taken_and_the_answer_says_so
  - src/use_cases/infobase_owner.rs::a_written_marker_passes_its_schema
---

# Метка называет каждую копию-владельца

Метка держит для каждой копии-владельца машину и каталог проекта; согласия делить базу в ней
нет (#437).

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
