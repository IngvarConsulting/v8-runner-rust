---
id: INV.CONFIG.LOCAL-PATH-HOLDS-THE-PERSONAL-STATE
check:
  - src/config/loader.rs::local_path_defaults_beside_the_project_file_and_the_local_layer_wins
  - src/config/loader.rs::a_local_path_over_the_project_a_set_or_the_work_path_is_refused_and_creates_nothing
  - src/config/loader.rs::a_local_path_over_the_standalone_exchange_directory_is_refused
  - src/config/loader.rs::a_local_path_spelled_in_another_case_is_the_same_directory
  - src/use_cases/config_init.rs::a_local_path_over_the_config_directory_gives_no_default_origin
  - src/use_cases/config_init.rs::a_local_path_over_the_config_directory_in_another_case_gives_no_default_origin
  - src/use_cases/config_init.rs::a_local_path_through_a_link_to_the_config_directory_gives_no_default_origin
  - src/platform/service_record.rs::a_stand_service_lives_under_the_local_path
  - src/use_cases/config_init.rs::the_default_origin_lies_in_the_declared_local_path
  - src/use_cases/config_init.rs::a_declared_project_keeps_its_project_file_and_declares_the_default_origin
---

# `localPath` держит личное состояние рабочей копии

`localPath` — необязательный ключ верхнего уровня, по умолчанию `.local`. Относительный путь
считается от каталога `v8project.yaml`, как у `workPath`. Ключ принимают и проектный файл, и
местный слой; местный слой главнее. `--workdir` на него не влияет, своего ключа командной строки
у него нет.

Под `localPath` лежит то, что нельзя пересобрать: память о базе в
`<localPath>/infobases/<база>/` (`INV.USE-CASES.MEMORY-LIVES-UNDER-THE-BASE-IT-DESCRIBES`),
данные стендов в `<localPath>/infobases/<база>/managed/{postgres,standalone,cluster,ras}/` и
файловая база,
которую `init` без `--infobase` объявляет по умолчанию, — `File=<localPath>/db`. Путь базы
пишется относительным, если он внутри каталога конфигурации, иначе абсолютным. Объявленный
`origin`, например `File=build/ib` прежнего проекта, не меняется.

Проверка настроек отказывает, если `localPath`:

- совпадает с каталогом конфигурации или содержит его;
- содержит корень любого набора исходников;
- пересекается с `workPath` в любую сторону;
- пересекается с `standalone.exchange.dir` любой базы.

Проверка сравнивает канонические пути — ссылки разрешены по ближайшему существующему предку, на
Windows регистр не различается — и ничего не создаёт на диске, в том числе в превью. `init`, который
выбирает базу по умолчанию до загрузки настроек, сравнивает так же и отказывает тем же текстом.
