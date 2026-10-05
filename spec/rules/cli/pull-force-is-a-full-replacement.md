---
id: INV.CLI.PULL-FORCE-IS-A-FULL-REPLACEMENT
check:
  - tests/cli_dump.rs::an_explicit_request_replaces_the_directory_and_keeps_nothing
  - tests/cli_synonyms.rs::pull_help_says_what_each_form_does
  - tests/cli_help.rs::pull_help_says_every_edt_dump_replaces_the_project
---

# `pull --force` — полная выгрузка с заменой каталога

`pull --force` выгружает конфигурацию целиком и заменяет каталог набора состоянием базы:
незафиксированное там пропадает без копии. Справка `pull` называет каждую форму, которая
заменяет каталог, вместе с этим последствием: `--force` — всегда, а в проекте EDT — любую
выгрузку. Решение принято в #191.
