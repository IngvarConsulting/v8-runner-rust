---
id: INV.USE-CASES.A-WITHDRAWN-CONSENT-STOPS-EVERY-COPY
check:
  - tests/cli_infobase_owner.rs::a_consent_withdrawn_on_this_machine_stops_every_copy_at_once
  - src/use_cases/infobase_owner.rs::a_remote_copy_that_withdrew_consent_stops_this_machine_after_its_next_write
---

# Отозванное согласие останавливает все копии

Если согласие отозвала одна из рабочих копий общей базы, команды записи всех её копий
отказывают и называют друг друга. Команда записи определена в
`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
