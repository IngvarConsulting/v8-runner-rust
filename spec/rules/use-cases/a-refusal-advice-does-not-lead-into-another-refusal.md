---
id: INV.USE-CASES.A-REFUSAL-ADVICE-DOES-NOT-LEAD-INTO-ANOTHER-REFUSAL
check:
  - tests/cli_dump.rs::an_edt_refusal_advises_a_full_replacement_that_runs_as_written
  - tests/cli_synonyms.rs::mode_full_with_object_advises_dropping_the_object_too
  - tests/cli_convert.rs::a_convert_refusal_does_not_offer_a_truncated_command
  - tests/cli_help.rs::pull_help_says_every_edt_dump_replaces_the_project
---

# Совет выхода из отказа сторожа не ведёт в новый отказ

Выход из отказа сторожа замены, названный отказом или справкой `pull`, выполненный
буквально, не упирается в другой отказ того же вызова.

Для `pull` это значит: совет — точная команда `pull <SET> --force` без `--object` и
прежнего `--mode`, а не «тот же вызов с `--force`», потому что `--force` рядом с ними
отказывает. Совет прямо говорит, что это полная выгрузка с заменой каталога набора. У
`convert` `--force` ни с чем в вызове не спорит, и совет — тот же вызов с добавленным
`--force`.
