---
id: INV.CLI.A-POSITIONAL-ARGUMENT-NEVER-NAMES-A-BASE
check:
  - tests/cli_positional_source_set.rs::a_positional_argument_names_a_source_set_and_never_a_base
  - tests/cli_positional_source_set.rs::a_positional_source_set_selects_that_set
  - tests/cli_bootstrap.rs::clone_takes_its_source_from_the_from_key
  - tests/cli_config_init.rs::init_writes_the_address_named_by_the_global_key_into_origin
---

# Позиционный аргумент никогда не называет базу

Позиционный аргумент ни одной команды не разбирается как имя базы или строка соединения;
у команд, принимающих набор исходников, позиционный — набор, и чужое значение на этом
месте — отказ, а не догадка. Базу называет ключ: `--infobase`, а источник у
`clone` — `--from`.
