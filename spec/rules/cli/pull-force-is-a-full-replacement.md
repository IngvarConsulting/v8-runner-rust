---
id: INV.CLI.PULL-FORCE-IS-A-FULL-REPLACEMENT
check:
  - tests/cli_synonyms.rs::a_mode_that_contradicts_force_is_refused_with_the_choice
  - tests/cli_synonyms.rs::pull_help_says_what_each_form_does
  - src/cli/execute.rs::a_previous_pull_mode_means_no_key_and_full_names_force
---

# `pull --force` — полная выгрузка с заменой каталога

`pull --force` выгружает конфигурацию целиком и заменяет каталог набора состоянием базы;
незафиксированное там пропадает, и справка `pull` говорит это сама. Другим режимом
`--force` не становится: рядом с `--object` он отказывает до платформы и называет выбор.

Решение владельца 05.10.2026, #191: `pull --force` — полная выгрузка уже в волне 1; #217
добавляет «слить» и отказ без `--force`.
