---
id: INV.USE-CASES.AN-AGENT-WRITES-A-DUMP-ONLY-THROUGH-A-LINK
check:
  - src/use_cases/agent_session.rs::an_output_directory_without_a_link_is_refused_and_not_copied
  - src/use_cases/agent_session.rs::a_write_through_the_exposed_output_directory_lands_in_the_target
  - src/use_cases/agent_session.rs::a_junction_carries_writes_and_its_removal_keeps_the_target
---

# Агент пишет выгрузку только через ссылку

Выгрузка по изменившемуся и выборка объектов пишут прямо в каталог набора: агент видит его
ссылкой внутри своего каталога. Копия здесь недопустима: агент обновил бы её, успешный ответ не
принёс бы изменений в файлы, а поколение было бы записано. Поэтому без ссылки `pull` отказывает
родом `environment` и называет другой путь.

На Windows без права на символические ссылки используется точка соединения: права она не требует
([замер](../../../references/1c/confirmed-runtime-measurements.md#агент-и-точка-соединения-windows)).
То, что агент только читает, можно выставить и копией, если ссылка невозможна.
