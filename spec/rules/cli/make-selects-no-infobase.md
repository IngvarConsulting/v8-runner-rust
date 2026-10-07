---
id: INV.CLI.MAKE-SELECTS-NO-INFOBASE
check:
  - tests/cli_make_download_all.rs::make_needs_no_infobase_and_refuses_the_infobase_key
  - tests/cli_global_flags.rs::a_leaf_that_selects_no_base_refuses_the_base_key
  - tests/cli_agent_standalone.rs::extensions_go_through_the_gate_and_make_never_does
---

# `make` базу проекта не выбирает

`make` собирает пакет во временной базе раннера и базу проекта не выбирает: проект без
местного слоя и без `origin` собирает пакет, а `--infobase` у `make` — отказ валидации.
Шлюз автономного сервера `make` не открывает.
