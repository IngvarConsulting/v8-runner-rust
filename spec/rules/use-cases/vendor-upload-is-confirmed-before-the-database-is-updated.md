---
id: INV.USE-CASES.VENDOR-UPLOAD-IS-CONFIRMED-BEFORE-THE-DATABASE-IS-UPDATED
check:
  - src/use_cases/load_artifact.rs::update_requires_readable_settings_with_the_platform_xml_structure
  - tests/cli_load.rs::vendor_update_preview_names_masked_argv_without_running_designer
  - tests/cli_load.rs::vendor_update_requires_a_version_change_before_updating_the_database
---

# Обновление поставщика подтверждается перед применением к базе

`upload --mode update` передаёт в `/UpdateCfg` файл поставки `.cf` или обновление
`.cfu` для основной конфигурации на поддержке. Подготовка читает и проверяет XML
`--settings` до запуска платформы. Конфигуратор получает временную копию проверенного
содержимого. Превью называет аргументы и возвращается до запуска.

Рабочая версия читается до и после `/UpdateCfg`. Только смена версии подтверждает
изменение, после которого разрешён отдельный `/UpdateDBCfg`. Неизменная или
непрочитанная версия после успешного `/UpdateCfg` оставляет результат
неподтверждённым и запрещает применение к базе. Нулевой код Конфигуратора сам по себе
недостаточен, в том числе при `DoNotMerge`.
