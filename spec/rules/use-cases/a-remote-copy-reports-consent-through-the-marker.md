---
id: INV.USE-CASES.A-REMOTE-COPY-REPORTS-CONSENT-THROUGH-THE-MARKER
check:
  - tests/cli_infobase_owner.rs::a_remote_copy_consents_through_the_marker
  - src/use_cases/infobase_owner.rs::a_remote_copy_that_withdrew_consent_stops_this_machine_after_its_next_write
  - src/use_cases/infobase_owner.rs::only_a_run_of_a_write_reports_consent
  - src/use_cases/infobase_owner.rs::a_sole_owner_records_its_changed_consent
---

# Копия с другой машины сообщает согласие меткой

Рабочая копия с другой машины сообщает согласие меткой: уже записанная в ней копия обновляет
его при каждой своей команде записи, взявшей замок базы, в том числе при отказе по
владельцу. Отзыв виден после её следующей такой команды. Команды записи и чтения определены в
`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
