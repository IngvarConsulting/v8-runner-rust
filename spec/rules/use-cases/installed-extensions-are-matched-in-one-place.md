---
id: INV.USE-CASES.INSTALLED-EXTENSIONS-ARE-MATCHED-IN-ONE-PLACE
check:
  - tests/architecture_guardrails.rs::installed_extensions_are_matched_in_one_place
  - tests/cli_make_download_all.rs::download_refuses_a_set_whose_sources_name_another_installed_extension
  - tests/cli_pull_all.rs::a_set_holding_an_extension_under_another_name_gets_no_second_set
---

# Наборы с составом базы сопоставляет одно место

`pull --all` и `download` без набора читают состав базы одним читателем
(`installed_extensions::read_installed_extensions`) и сопоставляют наборы проекта с ним одним
методом (`SourceSetInventory::installed_packages`): по имени расширения без учёта регистра.
Набор расширения, исходники которого называют другое установленное расширение, — отказ и `pull --all`, и
`download` до первой выгрузки (#218).
