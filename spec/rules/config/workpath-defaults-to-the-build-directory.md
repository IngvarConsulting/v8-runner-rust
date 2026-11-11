---
id: INV.CONFIG.WORKPATH-DEFAULTS-TO-THE-BUILD-DIRECTORY
check:
  - src/config/loader.rs::work_path_defaults_to_the_build_directory_beside_the_project_file
  - tests/cli_config_init.rs::config_init_creates_yaml_with_detected_designer_sources
---

# `workPath` по умолчанию — `.build/v8-runner`

Ключ `workPath` необязателен. Без него рабочий каталог — `.build/v8-runner` от каталога
`v8project.yaml`: `.build` делят инструменты рабочей копии, у раннера в нём свой подкаталог.
Явный ключ проектного файла или местного слоя и `--workdir` умолчание заменяют. `init` и
`clone` ключа не пишут.

В `workPath` лежит только то, что раннер считает заново: журналы, отчёты, временные файлы и
базы, промежуточный каталог выгрузки, кеши EDT и внешних артефактов, снимки Конфигуратора
наборов EDT. Память о базе в нём не лежит
(`INV.USE-CASES.MEMORY-LIVES-UNDER-THE-BASE-IT-DESCRIBES`).
