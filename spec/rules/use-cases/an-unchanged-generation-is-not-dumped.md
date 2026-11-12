---
id: INV.USE-CASES.AN-UNCHANGED-GENERATION-IS-NOT-DUMPED
check:
  - tests/cli_dump_agent.rs::an_unchanged_generation_skips_an_incremental_dump
  - tests/cli_push_generation.rs::a_designer_pull_with_an_unchanged_generation_dumps_nothing
  - tests/cli_push_generation.rs::an_ibcmd_pull_with_an_unchanged_generation_dumps_nothing
  - tests/cli_push_generation.rs::a_second_pull_before_a_commit_with_an_unchanged_generation_is_up_to_date
  - tests/cli_push_generation.rs::an_uncommitted_edit_with_a_changed_generation_is_refused_by_the_guard
  - tests/cli_dump_agent.rs::an_unchanged_generation_answers_up_to_date_over_uncommitted_work
---

# Неизменившееся поколение не выгружается

Выгрузка по изменившемуся по годной копии описи раннера — агентом, Конфигуратором или `ibcmd`, —
увидев до выгрузки поколение, равное записанному после последней загрузки или выгрузки
инструментом, чей токен с его токеном сравним
(`INV.USE-CASES.A-GENERATION-TOKEN-IS-COMPARED-WITHIN-ITS-OWN-TOOL`), отвечает успехом с признаком «всё актуально» (`up_to_date`) и
сообщением и выгрузку не запускает. Так чтение поколения, которое выгрузка делает всё равно,
окупается.

Поколение сверяется раньше сторожа незакоммиченной работы
(`INV.USE-CASES.REPLACING-A-USER-DIRECTORY-ASKS-FIRST`), одинаково у всех исполнителей:
неизменившееся поколение отвечает `up_to_date` и дерево не трогает, даже если в каталоге
набора есть незакоммиченное. Изменилось поколение или его нет — выгрузка будет писать, и
сторож спрашивается до неё, как прежде. Лишнего чтения поколения ради этого нет.

Полная выгрузка и выборка объектов по поколению не пропускаются: полная получает дерево, и
оно становится хеш-памятью базы, а журнал поколения не подтверждает, что локальные файлы
совпадают с базой.

Решение владельца от 06.10.2026: Конфигуратор и `ibcmd` пропускают выгрузку так же, как
агент.
