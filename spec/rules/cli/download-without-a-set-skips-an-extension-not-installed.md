---
id: INV.CLI.DOWNLOAD-WITHOUT-A-SET-SKIPS-AN-EXTENSION-NOT-INSTALLED
check:
  - tests/cli_make_download_all.rs::download_without_a_set_downloads_the_installed_packages
---

# `download` без набора берёт расширения, которые есть в базе

`download` без набора спрашивает базу о расширениях выбранным исполнителем `download` тем же
читателем, что `pull --all`, и выгружает набор расширения, только если его расширение в базе
есть; имена сравниваются без регистра. Набор, чьего расширения нет, не выгружается и назван в
`data.not_installed`. Расширения базы без набора команда не выгружает и не объявляет.
