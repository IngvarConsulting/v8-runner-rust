---
id: INV.CLI.INFOBASE-CREATE-FOLLOWS-THE-TARGET-KIND
check:
  - tests/cli_init.rs::a_file_base_is_created_by_ibcmd_with_the_main_configuration_and_remembers_it
  - tests/cli_init.rs::init_designer_creates_infobase_and_skips_edt_workspace
  - tests/cli_init.rs::a_cluster_base_is_created_by_the_designer_with_the_client_server_string
  - tests/cli_init.rs::a_cluster_base_without_a_required_dbms_field_is_refused_before_the_platform_starts
  - tests/cli_init.rs::a_standalone_target_is_refused_with_the_recipe
  - tests/cli_init.rs::an_existing_file_base_is_refused_and_the_preview_names_it
  - tests/cli_init.rs::a_failed_ibcmd_create_that_left_a_base_names_the_directory_and_the_ways_out
  - tests/cli_init.rs::a_cluster_preview_names_the_target_and_starts_nothing
  - tests/cli_init.rs::init_edt_imports_projects_in_configuration_then_extension_order
  - tests/cli_init.rs::a_file_base_of_an_edt_project_is_assembled_by_ibcmd_from_its_sources
  - tests/cli_push_generation.rs::a_base_created_by_the_runner_takes_the_first_push
  - tests/cli_infobase_owner.rs::infobase_create_records_the_created_base_for_this_copy
  - src/use_cases/init_project.rs::a_cancellation_deferred_by_the_creation_leaves_an_empty_remembered_base
---

# Базу создаёт `infobase create`, и делает это по виду цели

Без `--from` файловую базу `ibcmd` создаёт сразу с основной конфигурацией из исходников (`infobase create --import --apply --force`); запасной
исполнитель — Конфигуратор: `CREATEINFOBASE`, затем загрузка основной конфигурации и
обновление базы данных. В кластере Конфигуратор одной командой `CREATEINFOBASE` с
клиент-серверной строкой регистрирует базу и создаёт базу данных в СУБД, всегда с запретом
регламентных заданий (`SchJobDn=Y`); реквизиты берутся
из секции `dbms`, включая `locale`, и из `cluster.user` и `cluster.password`. Без
обязательного реквизита `dbms` — отказ до запуска платформы с именем ключа. Автономный сервер
получает отказ рода подбора с рецептом `ibcmd server config init` и `ibcmd infobase create`.
Превью базы в кластере предупреждает, что `CrSQLDB=Y` молча берёт существующую базу данных
с тем же именем, даже с чужой базой, и что неудача может оставить базу данных брошенной.

Для формата EDT команда создаёт ещё и рабочую область. Созданная база сразу записывается в
память: у файловой — набор, из которого она собрана, и первая отправка досылает остальное;
у базы в кластере, которую Конфигуратор создаёт пустой, и у файловой, чью сборку остановили
после создания, — только то, что база есть, и первая отправка полная. Созданную файловую
базу команда записывает в метку за своей копией. Существующая файловая база — отказ на тех
же правах, что у подъёма из снимка с созданием. Неудачное создание, после которого файл базы
всё же появился, называет оставленный каталог и выходы: удалить его и создать базу заново или
загрузить исходники поверх `push --force`. Копию другой базы делает `--from`
(`INV.CLI.INFOBASE-CREATE-FROM-COPIES-A-BASE`).

Исходники проекта EDT сперва переводятся в XML
(`INV.CLI.A-FILE-BASE-OF-AN-EDT-PROJECT-IS-ASSEMBLED-FROM-ITS-SOURCES`). Чего правило не
держит, держат правила с разрывом: запасной путь `rac` (`INV.CLI.A-CLUSTER-BASE-FALLS-BACK-TO-RAC`) и отказ на существующей базе в кластере
(`INV.CLI.AN-EXISTING-CLUSTER-BASE-IS-REFUSED-BEFORE-CREATION`).

Источник: [`cli.html#map`](../../../docs/site/cli.html#map),
[`sources.html#copies`](../../../docs/site/sources.html#copies), замер
[#181](../../../references/1c/confirmed-runtime-measurements.md).
