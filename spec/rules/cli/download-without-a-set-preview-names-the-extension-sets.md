---
id: INV.CLI.DOWNLOAD-WITHOUT-A-SET-PREVIEW-NAMES-THE-EXTENSION-SETS
check:
  - tests/cli_make_download_all.rs::download_without_a_set_preview_reads_nothing
---

# Превью `download` без набора называет наборы расширений

Превью `download` без набора платформу не запускает и состава базы не знает. Превью выгрузки
получают наборы конфигурации — их прогон выгрузит при любом составе базы; наборы расширений
проекта названы в `data.if_installed` без превью выгрузки. Ничего не пишется.
