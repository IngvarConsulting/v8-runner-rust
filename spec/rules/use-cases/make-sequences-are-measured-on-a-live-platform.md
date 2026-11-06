---
id: INV.USE-CASES.MAKE-SEQUENCES-ARE-MEASURED-ON-A-LIVE-PLATFORM
check:
  - src/use_cases/artifacts.rs::designer_builds_a_cf_from_the_sources_in_the_build_infobase
  - src/use_cases/artifacts.rs::ibcmd_imports_and_saves_the_working_configuration_with_its_own_data_directory
  - src/use_cases/artifacts.rs::an_external_set_is_built_on_top_of_the_configuration
---

# Последовательности `make` замерены на живой платформе

Замеры на 8.3.27.2074 подтверждают создание файловой базы, полную и частичную
загрузку XML и сохранение пакета из рабочей конфигурации у Конфигуратора и
`ibcmd`. Пакеты из частичной и полной загрузок равны по содержимому.
Конфигуратор идёт первым в цепочке `make` по решению владельца #472 на основе
замера #471. Сборка внешних обработок замерена в базе с загруженной основной
конфигурацией. Подробности в
[замерах платформы](../../../references/1c/confirmed-runtime-measurements.md).
