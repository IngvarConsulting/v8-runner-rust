---
id: INV.CLI.PULL-FORCE-TAKES-NO-OBJECT
check:
  - tests/cli_synonyms.rs::object_next_to_force_is_refused_with_the_choice
  - src/cli/execute.rs::a_previous_pull_mode_means_no_key_and_full_names_force
---

# `pull --object` рядом с `--force` отказывает

`--force` — полная выгрузка, `--object` — частичная, и вместе они не исполняются: вызов
отказывает до платформы (`invalid_argument`, выход 2) и называет выбор — оставить
`--object` ради названных объектов или оставить `--force` ради полной замены каталога.
Частичной выгрузки с согласием на уничтожение командная строка не даёт.
