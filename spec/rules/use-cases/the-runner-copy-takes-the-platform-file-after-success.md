---
id: INV.USE-CASES.THE-RUNNER-COPY-TAKES-THE-PLATFORM-FILE-AFTER-SUCCESS
check:
  - tests/cli_pull_memory.rs::a_version_file_in_the_sources_never_reaches_the_dump
  - tests/cli_pull_memory.rs::a_failed_pull_does_not_change_the_runner_copy
  - tests/cli_pull_memory.rs::a_designer_partial_pull_leaves_the_runner_copy_alone
  - tests/cli_pull_memory.rs::an_ibcmd_partial_pull_dumps_from_the_runner_copy
  - tests/cli_pull_memory.rs::a_failed_copy_write_is_a_warning_not_a_refusal
  - src/use_cases/dump_config.rs::a_broken_overlay_keeps_the_runner_copy_as_it_was
---

# Копия раннера перенимает опись платформы только после удачи

Копия описи раннера (`INV.USE-CASES.THE-VERSION-FILE-LIVES-ONLY-IN-THE-RUNNER-MEMORY`)
перенимает опись, которую платформа написала в стадию, только после удачной выгрузки: полной —
после публикации дерева, по изменившемуся и выборки `ibcmd` — после удалений по двум описям и
переноса написанного в каталог набора. Сбой платформы, удалений или переноса оставляет копию
прежней: она старше дерева, и следующая выгрузка по ней выгрузит больше, а не меньше.
Выборочная выгрузка Конфигуратора и агента копию не меняет. Загрузка `push` меняет её, только
когда получает свежую опись — после полной загрузки или крупной по изменившемуся, если поколение
во время выгрузки описи не сдвинулось
(`INV.USE-CASES.A-PUSH-REFRESHES-THE-VERSION-FILE-ONLY-WHEN-IT-PAYS`).

Если копию не удалось записать после удачной команды, ответ остаётся удачным и несёт
предупреждение: прежняя копия ведёт к выгрузке лишнего, а не к пропуску изменений.
