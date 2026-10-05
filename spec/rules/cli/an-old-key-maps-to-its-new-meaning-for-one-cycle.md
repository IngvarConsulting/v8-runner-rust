---
id: INV.CLI.AN-OLD-KEY-MAPS-TO-ITS-NEW-MEANING-FOR-ONE-CYCLE
check:
  - tests/cli_synonyms.rs::every_previous_name_answers_as_its_dictionary_entry
  - tests/cli_synonyms.rs::mode_full_is_refused_and_names_pull_force
  - src/cli/execute.rs::a_previous_pull_mode_means_no_key_and_full_names_force
  - tests/cli_positional_source_set.rs::download_accepts_the_hidden_state_values_and_help_hides_them
---

# Прежний ключ без пары отображается в новый смысл на один цикл

Ключи, у которых нет пары с новым именем, принимаются один цикл выпуска так:
`--source-set <имя>` — синоним позиционного набора; `--state working` — то же, что без
ключа; `--state database` — `--state db`; `--mode incremental` и `--mode partial` — без
ключа, режим выбирает наличие `--object`.

Прежний режим не отображается в замену каталога: `--mode full` отказывает и называет
`pull --force` вместе с тем, что он делает.
