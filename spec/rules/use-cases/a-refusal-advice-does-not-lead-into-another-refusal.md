---
id: INV.USE-CASES.A-REFUSAL-ADVICE-DOES-NOT-LEAD-INTO-ANOTHER-REFUSAL
check:
  - tests/cli_dump.rs::an_edt_refusal_advises_a_full_replacement_that_runs_as_written
  - tests/cli_synonyms.rs::mode_full_with_object_advises_dropping_the_object_too
  - tests/cli_synonyms.rs::mode_full_next_to_force_advises_keeping_force
  - tests/cli_synonyms.rs::a_mode_with_object_next_to_force_advises_dropping_the_object_too
  - src/cli/execute.rs::a_previous_pull_mode_means_no_key_and_full_names_force
  - tests/cli_help.rs::pull_help_says_every_edt_dump_replaces_the_project
---

# Совет выхода из отказа не ведёт в новый отказ

Выход к полной замене каталога, названный отказом `pull` или его справкой, выполненный
буквально, не упирается в другой отказ того же вызова.

Для отказа сторожа замены это значит: совет — точная команда `pull <SET> --force` без
`--object` и прежнего `--mode`, а не «тот же вызов с `--force`», потому что `--force`
рядом с ними отказывает. Совет прямо говорит, что это полная выгрузка с заменой каталога
набора. Отказ прежнего `--mode` рядом с `--object` велит снять и каждый `--object`, а при
уже стоящем `--force` — оставить его, а не добавить второй, который не примет разбор
ключей.
