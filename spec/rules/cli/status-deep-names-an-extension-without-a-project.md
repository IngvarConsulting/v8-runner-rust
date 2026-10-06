---
id: INV.CLI.STATUS-DEEP-NAMES-AN-EXTENSION-WITHOUT-A-PROJECT
check:
  - tests/cli_status.rs::status_deep_names_an_extension_without_a_project
---

# `status --deep` называет расширение базы без набора в проекте

`status --deep` читает состав расширений базы и сопоставляет его с наборами расширений
проекта по имени без учёта регистра. Расширение базы без набора отвечает `source_set: null`,
набор проекта без расширения в базе стоит в `missing_in_base`.

Источник: [`cli.html`](../../../docs/site/cli.html), раздел «У расширения в базе без проекта
есть имя».
