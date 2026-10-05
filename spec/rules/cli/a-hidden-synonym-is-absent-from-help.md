---
id: INV.CLI.A-HIDDEN-SYNONYM-IS-ABSENT-FROM-HELP
check:
  - tests/cli_help.rs::no_help_at_any_level_prints_a_previous_name
  - src/cli/args.rs::the_synonym_table_names_every_hidden_name_of_the_parser
  - tests/cli_help.rs::pull_and_make_help_name_the_source_set_positionally
  - tests/cli_help.rs::upload_help_shows_the_package_file_as_required_positional
  - tests/cli_positional_source_set.rs::download_accepts_the_hidden_state_values_and_help_hides_them
---

# Скрытый синоним отсутствует в справке

`--help` любого уровня показывает только словарь сайта: прежнее имя команды или ключа
принимается, но в справке не печатается.
