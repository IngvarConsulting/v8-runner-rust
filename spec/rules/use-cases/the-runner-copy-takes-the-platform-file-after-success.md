---
id: INV.USE-CASES.THE-RUNNER-COPY-TAKES-THE-PLATFORM-FILE-AFTER-SUCCESS
check:
  - tests/cli_pull_memory.rs::a_foreign_version_file_between_commands_does_not_reach_the_dump
  - tests/cli_pull_memory.rs::a_failed_pull_does_not_change_the_runner_copy
  - tests/cli_pull_memory.rs::a_failed_push_does_not_change_the_runner_copy
  - tests/cli_pull_memory.rs::a_push_refreshes_the_runner_copy
  - tests/cli_pull_memory.rs::a_push_that_writes_no_version_file_keeps_the_runner_copy
  - tests/cli_pull_memory.rs::a_designer_partial_pull_leaves_the_runner_copy_alone
  - tests/cli_pull_memory.rs::an_agent_push_loads_over_the_runner_copy_and_records_the_new_one
  - tests/cli_pull_memory.rs::a_failed_copy_write_is_a_warning_not_a_refusal
  - src/use_cases/version_file.rs::a_load_that_did_not_rewrite_the_file_keeps_the_copy
---

# Копия раннера перенимает файл платформы только после удачи

Файл версий в каталоге набора остаётся там после команды. Копия раннера
(`INV.USE-CASES.A-REPLACED-VERSION-FILE-GIVES-WAY-TO-THE-RUNNER-COPY`) перенимает его после
удачной полной выгрузки, удачной выгрузки по изменившемуся и удачной выборки `ibcmd`, а
после удачной загрузки `push` — только если загрузка переписала файл в каталоге. Загрузка,
которая файла в каталоге не переписала, копию не меняет: так у `ibcmd config import` и у
агента, которому каталог передан копией (по SFTP или в общий каталог без ссылки).
Выборочная выгрузка Конфигуратора копию не меняет. Сбой команды оставляет копию прежней.

Если копию не удалось записать после удачной команды, ответ остаётся удачным и несёт
предупреждение: прежняя копия ведёт к выгрузке лишнего, а не к пропуску изменений.
