---
id: INV.USE-CASES.A-LOAD-IS-PARTIAL-UNLESS-NOTHING-TO-COMPARE
check:
  - src/change_detection/partial_load.rs::decide_loads_changed_configuration_xml_partially
  - src/change_detection/partial_load.rs::decide_includes_existing_payloads_of_an_added_object
  - src/change_detection/partial_load.rs::agent_root_dependencies_include_declared_payloads_only
  - tests/cli_agent_standalone.rs::a_partial_root_load_through_sftp_stages_dependencies_outside_the_load_list
  - src/change_detection/partial_load.rs::decide_expands_changed_directory_to_files_with_descriptors
  - src/change_detection/partial_load.rs::decide_loads_root_instead_of_deleted_object_files
  - src/change_detection/partial_load.rs::decide_keeps_ambiguous_and_retained_owner_deletions_full
  - src/change_detection/partial_load.rs::a_removed_object_with_changed_descendants_requires_full_load
  - src/change_detection/partial_load.rs::decide_loads_more_than_twenty_changed_files_partially
  - src/platform/ibcmd.rs::config_import_partial_reports_oversized_argv_without_retries
  - tests/cli_push_generation.rs::a_base_created_by_the_runner_takes_the_first_push
---

# Загрузка частичная, если есть с чем сравнить

`push` загружает только изменённое, когда у набора есть снимок файлов. Правка корневого
описания `Configuration.xml`, новый объект, удаление целого объекта и изменение каталога
включают в список `Configuration.xml`. Для нового объекта список включает его файлы.
Удалённые пути в список не входят. Удаление целого объекта подтверждается исчезновением
его объявления из корневого описания. Если объявление осталось или удалён только
подчинённый файл, нужна полная загрузка: частичная загрузка XML владельца сохраняет
удалённый модуль, справку и содержимое макета.

Конфигуратор и агент получают файл со списком. Для агента по SFTP корень в списке
требует передать также файлы объявленных объектов. Они доступны платформе как зависимости
и не добавляются в список загрузки. `ibcmd` принимает файлы позиционными
аргументами и при отказе ОС запустить слишком длинную команду сообщает о лимите.
Порога числа файлов в раннере нет. Список не делится на порции, чтобы сохранить
атомарность операции под хранилищем.
[Замеры](../../../references/1c/confirmed-runtime-measurements.md#частичная-загрузка-структурных-изменений).

Полная загрузка остаётся при пустом снимке файлов, по `push --full` и `push --force`,
при первой отправке после создания базы в кластере
(`INV.CLI.INFOBASE-CREATE-FOLLOWS-THE-TARGET-KIND`) и после копии
(`INV.USE-CASES.A-COPIED-BASE-STARTS-WITH-A-FULL-PUSH`), для расширений формата EDT,
при небезопасном пути
(`INV.USE-CASES.A-CHANGE-OUTSIDE-THE-SOURCE-ROOT-TURNS-THE-LOAD-FULL`),
неподтверждённом удалении и восстановимом сбое анализа.
