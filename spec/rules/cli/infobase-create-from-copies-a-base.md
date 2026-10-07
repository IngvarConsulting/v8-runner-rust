---
id: INV.CLI.INFOBASE-CREATE-FROM-COPIES-A-BASE
check:
  - tests/cli_infobase_copy.rs::debugging_on_a_copy_of_the_base_leaves_the_neighbour_untouched
  - tests/cli_infobase_copy.rs::a_cluster_copy_is_created_by_the_designer_and_loaded_from_the_image
  - tests/cli_infobase_copy.rs::a_preview_names_the_snapshot_and_a_wrong_source_is_refused
  - src/platform/ibcmd.rs::a_base_from_an_image_is_restored_with_the_measured_create_key
---

# `infobase create --from` копирует базу

`infobase create --from <база>` создаёт базу этой рабочей копии как копию другой базы,
объявленной по имени в местном слое, с её данными и конфигурацией: снимает с источника образ
DT Конфигуратором (`/DumpIB`) в `workPath/copies/<база>.dt` и создаёт из него файловую базу
через `ibcmd infobase restore --create-database`, базу в кластере — Конфигуратором:
`CREATEINFOBASE`, затем `/RestoreIB` образа. Новую базу на автономном сервере команда не
создаёт: отказ называет рецепт. Источник, не объявленный в местном слое, и база, которую
команда создаёт, — отказ до платформы.

Источник читается как база целиком: замок источника берётся на время снимка, в его метку
копия не пишется (`INV.USE-CASES.READING-A-BASE-MAKES-NO-OWNER`), а новая база записывается
в метку за этой копией. Ответ называет источник и путь снимка
([форма](../wire/infobase-create-data.md)). Шаблон .dt у `CREATEINFOBASE` в кластере ждёт
замера (`INV.CLI.A-CLUSTER-COPY-IS-CREATED-FROM-THE-DT-TEMPLATE`).

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies),
[`cli.html#map`](../../../docs/site/cli.html#map), замер «Загрузка информационной базы из
DT» и [#181](../../../references/1c/confirmed-runtime-measurements.md).
