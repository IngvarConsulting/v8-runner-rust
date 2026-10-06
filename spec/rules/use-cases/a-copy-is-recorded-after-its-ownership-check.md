---
id: INV.USE-CASES.A-COPY-IS-RECORDED-AFTER-ITS-OWNERSHIP-CHECK
check:
  - tests/cli_infobase_owner.rs::a_write_on_a_base_of_another_copy_is_refused_and_names_the_owner
  - tests/cli_infobase_owner.rs::a_preview_names_the_ownership_refusal_and_writes_nothing
  - src/use_cases/infobase_owner.rs::only_a_run_of_a_write_on_a_declared_base_records_a_copy
  - src/use_cases/infobase_owner.rs::a_base_whose_lock_is_busy_keeps_its_marker
  - src/use_cases/infobase_owner.rs::a_machine_without_an_identity_is_not_recorded
---

# Копию в метку записывает команда, прошедшая проверку владельца

Свою рабочую копию в метку записывает команда записи на базе, названной в местном слое,
когда проверка владельца пройдена. Отказ по владельцу новую копию в метку не записывает;
превью и невзятый замок метку не меняют. Команды записи и чтения определены в
`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
