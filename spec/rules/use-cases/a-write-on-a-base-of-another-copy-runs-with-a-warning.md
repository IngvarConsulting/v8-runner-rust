---
id: INV.USE-CASES.A-WRITE-ON-A-BASE-OF-ANOTHER-COPY-RUNS-WITH-A-WARNING
check:
  - tests/cli_infobase_owner.rs::a_write_on_a_base_of_another_copy_runs_with_a_warning_and_names_the_owner
  - tests/cli_infobase_owner.rs::pull_all_on_a_base_of_another_copy_warns_and_names_the_owner
  - tests/cli_infobase_owner.rs::an_owner_on_another_machine_is_never_replaced
  - src/use_cases/transport.rs::a_base_of_another_copy_warns_before_the_scenario
  - src/use_cases/infobase_owner.rs::the_refusal_and_the_warning_name_the_same_ways_out
  - src/use_cases/infobase_owner.rs::a_marker_with_this_copy_and_another_live_one_warns
---

# Запись в базу другой копии идёт с предупреждением

Команда записи на базе другой рабочей копии
(`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`) не отказывает и
подтверждения не просит: она выполняется, а её ответ несёт предупреждение. Оно называет
копию-владельца, говорит, что команда меняет базу этой копии, называет метку, как освободить
базу (убрать её из `v8project.local.yaml` копии-владельца или удалить эту копию; у копии с
другой машины — удалить её запись из метки) и те же выходы к своей базе, что отказ
`INV.CLI.A-REFUSAL-WITHOUT-AN-OWN-BASE-NAMES-THE-WAYS-OUT`, с источником копии `upstream`. Кода
отказа для базы другой копии нет. Так же — когда в метке записана и эта копия: предупреждает
любая другая живая копия-владелец.

Решение владельца от 07.10.2026 (#437): писать в базу другой копии — ответственность
разработчика, и принятый ответ с предупреждением — его согласие.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
