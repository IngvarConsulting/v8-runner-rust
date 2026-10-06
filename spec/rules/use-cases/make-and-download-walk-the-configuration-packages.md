---
id: INV.USE-CASES.MAKE-AND-DOWNLOAD-WALK-THE-CONFIGURATION-PACKAGES
check:
  - tests/architecture_guardrails.rs::the_order_of_source_sets_is_decided_in_one_place
  - tests/cli_make_download_all.rs::make_without_a_set_builds_every_set_into_the_directory
  - tests/cli_make_download_all.rs::download_without_a_set_downloads_the_installed_packages
---

# `make` и `download` без набора обходят наборы порядком инвентаря

`download` без набора идёт по пакетам конфигурации проекта через
`SourceSetInventory::configuration_packages`, тем же порядком, что `pull --all`: основная
конфигурация, затем наборы расширений в порядке объявления. Наборы внешних файлов в его обход
не входят.

`make` без набора идёт по всем наборам проекта через
`SourceSetInventory::ordered_source_sets`: сперва пакеты конфигурации тем же порядком, затем
внешние обработки и внешние отчёты. Порядок каждого из этих обходов решает
`source_inventory::ordered_by_purpose`; свой порядок обход не заводит.
