---
id: INV.USE-CASES.THE-MARKER-IS-WRITTEN-ONLY-UNDER-THE-BASE-LOCK
check:
  - tests/cli_infobase_owner.rs::processes_racing_for_a_base_without_a_marker_leave_one_owner
  - src/use_cases/infobase_owner.rs::a_base_whose_lock_is_busy_keeps_its_marker
  - tests/architecture_guardrails.rs::the_owner_of_a_file_base_is_checked_in_one_place
---

# Метку пишут только под замком базы

Метку пишут только под замком файловой базы: из копий одной машины, начавших одновременно,
владельцем становится не больше одной.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
