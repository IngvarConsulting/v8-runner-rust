---
id: INV.CLI.PULL-ALL-SKIPS-A-SET-WHOSE-EXTENSION-IS-NOT-INSTALLED
check:
  - tests/cli_pull_all.rs::an_extension_without_a_set_is_declared_and_pulled
  - src/use_cases/dump_config/all.rs::the_walk_keeps_project_sets_and_declares_the_rest
---

# Набор расширения, которого нет в базе, не выгружается

Набор расширения проекта, чьего расширения в базе нет, `pull --all` не выгружает и называет
в `data.not_installed`. Набор и его каталог остаются: удалять наборы команда не берётся.
