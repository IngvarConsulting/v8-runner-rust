---
id: INV.CLI.MAKE-AND-DOWNLOAD-WITHOUT-A-SET-WRITE-INTO-A-DIRECTORY
check:
  - tests/cli_make_download_all.rs::make_without_a_set_builds_every_set_into_the_directory
  - tests/cli_make_download_all.rs::download_without_a_set_downloads_the_installed_packages
  - src/use_cases/source_inventory.rs::a_package_is_named_after_its_set
---

# Без набора `--output` — каталог, файл называется именем набора

`make` и `download` без позиционного набора и без `--extension` пишут пакет каждого набора
обхода в каталог, который называет `--output`: `<каталог>/<SET>.cf` у набора конфигурации,
`<каталог>/<SET>.cfe` у набора расширения и каталог `<каталог>/<SET>` у набора внешних
файлов в `make`. `download` считает относительный каталог от `basePath`, `make` — от
текущего каталога, как и с набором.

Прежде `make --output x.cf` и `download --output x.cf` без набора собирали и выгружали
основную конфигурацию; решение владельца (06.10.2026, #364) это поведение сняло без
синонима.
