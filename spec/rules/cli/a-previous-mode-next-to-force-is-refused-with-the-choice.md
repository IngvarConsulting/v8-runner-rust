---
id: INV.CLI.A-PREVIOUS-MODE-NEXT-TO-FORCE-IS-REFUSED-WITH-THE-CHOICE
check:
  - tests/cli_synonyms.rs::a_mode_that_contradicts_force_is_refused_with_the_choice
  - src/cli/execute.rs::a_previous_pull_mode_means_no_key_and_full_names_force
---

# Прежний режим рядом с `--force` отказывает и называет выбор

`pull --mode incremental --force` и `pull --mode partial --force` отказывают до платформы
(`invalid_argument`, выход 2). Отказ называет выбор: убрать `--mode` ради полной замены
каталога или убрать `--force` ради выгрузки без замены. Прежний режим не поднимается до
замены каталога молча: такое отображение обошло бы сторожа невосстановимой работы.
