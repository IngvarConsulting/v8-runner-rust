---
id: INV.CLI.DOWNLOAD-IN-A-PROJECT-WITHOUT-PACKAGES-TAKES-THE-MAIN-CONFIGURATION
check:
  - tests/cli_infobase.rs::infobase_export_does_not_require_project_source_sets
---

# `download` в проекте без пакетов выгружает основную конфигурацию в файл

У проекта без набора конфигурации и без набора расширения (`source-set: []`, только база)
обходить нечего: `download` без набора и без `--extension` выгружает основную конфигурацию в
файл `--output`, как до обхода наборов (#364), а не отказывает.
