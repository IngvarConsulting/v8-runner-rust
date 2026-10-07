---
id: INV.USE-CASES.A-WRITE-ON-A-BASE-OF-ANOTHER-COPY-RUNS-WITH-A-WARNING
check:
  - tests/cli_infobase_owner.rs::a_write_on_a_base_of_another_copy_runs_with_a_warning_and_names_the_owner
  - tests/cli_infobase_owner.rs::pull_all_on_a_base_of_another_copy_warns_and_names_the_owner
  - tests/cli_infobase_owner.rs::an_owner_on_another_machine_is_never_replaced
  - src/use_cases/transport.rs::a_base_of_another_copy_warns_before_the_scenario
---

# Запись в базу другой копии идёт с предупреждением

Команда записи на базе другой рабочей копии
(`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`) не отказывает и
подтверждения не просит: она выполняется, а её ответ несёт предупреждение. Оно называет
копию-владельца, говорит, что команда меняет базу этой копии, называет метку и выходы к своей
базе: копию этой базы с данными (`init --infobase`, затем `infobase create --from upstream`),
базу из эталонного образа (`init --infobase`, затем `infobase restore --input <образ>.dt
--create`) и голую базу из исходников (`init --infobase`, затем `infobase create`). Общую базу
выходом оно не называет. Кода отказа для базы другой копии нет.

Решение владельца от 07.10.2026 (#437): общих баз нет; писать в базу другой копии —
ответственность разработчика, и принятый ответ с предупреждением — его согласие.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
