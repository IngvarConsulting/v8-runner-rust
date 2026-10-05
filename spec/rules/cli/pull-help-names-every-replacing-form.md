---
id: INV.CLI.PULL-HELP-NAMES-EVERY-REPLACING-FORM
check:
  - tests/cli_synonyms.rs::pull_help_says_what_each_form_does
  - tests/cli_help.rs::pull_help_says_every_edt_dump_replaces_the_project
---

# Справка `pull` называет каждую форму, которая заменяет каталог

Справка `pull` называет каждую форму, которая заменяет каталог набора, вместе с
последствием — потерей незафиксированного: `--force` — всегда, а в проекте EDT — любую
выгрузку. Выход из отказа сторожа справка называет как ту же команду с добавленным
`--force`, а не как голый `pull --force`.
