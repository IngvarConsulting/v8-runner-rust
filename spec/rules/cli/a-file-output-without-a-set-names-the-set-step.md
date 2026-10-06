---
id: INV.CLI.A-FILE-OUTPUT-WITHOUT-A-SET-NAMES-THE-SET-STEP
check:
  - tests/cli_make_download_all.rs::a_file_output_without_a_set_is_refused_with_the_set_step
  - src/use_cases/source_inventory.rs::a_file_output_without_a_set_names_the_main_set
  - tests/cli_make_download_all.rs::without_a_configuration_set_a_file_output_names_no_step
---

# Файл в `--output` без набора — отказ с шагом к набору

`make` и `download` без набора отказывают до запуска платформы, если `--output` называет
файл: существующий файл или путь с суффиксом, который не является существующим каталогом.
Отказ — род `validation`, а `next` называет ту же команду с набором основной конфигурации и
`--output` с тем же путём и суффиксом `.cf`. Без набора конфигурации в проекте шага нет.
