---
id: INV.CLI.PACKAGE-NAMES-OF-A-WALK-NEITHER-COLLIDE-NOR-NAME-A-DEVICE
check:
  - tests/cli_make_download_all.rs::package_names_that_collide_or_name_a_device_are_refused_before_work
---

# Имена пакетов обхода не совпадают и не называют устройство

`make` и `download` без набора называют пакет именем набора: файл `<имя>.cf` или
`<имя>.cfe`, каталог `<имя>` у внешнего набора. Наборы, чьи имена файла или каталога пакета
совпадают без учёта регистра (`sales` и `Sales`, расширение `Sales` и внешний набор
`Sales.cfe`), и набор с именем устройства Windows (`CON`, `AUX` и подобные) — отказ родом
`validation` до запуска платформы: на файловой системе без регистра их пакеты легли бы по
одному пути, а файл с именем устройства в Windows не создать.
