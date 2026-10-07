---
id: INV.CLI.INFOBASE-CREATE-FROM-COPIES-A-BASE
check:
  - tests/cli_infobase_copy.rs::debugging_on_a_copy_of_the_base_leaves_the_neighbour_untouched
  - tests/cli_infobase_copy.rs::a_cluster_copy_is_created_by_the_designer_and_loaded_from_the_image
  - tests/cli_infobase_copy.rs::a_cluster_copy_preview_warns_that_an_existing_database_is_overwritten
  - tests/cli_infobase_copy.rs::a_preview_names_the_snapshot_and_a_wrong_source_is_refused
  - tests/cli_infobase_copy.rs::a_source_held_by_a_running_command_is_refused_before_the_snapshot
  - tests/cli_infobase_copy.rs::the_source_stays_locked_while_the_copy_runs
  - tests/cli_infobase_copy.rs::a_copy_the_runner_cannot_open_names_the_users_of_the_source
  - tests/cli_infobase_copy.rs::a_copy_into_a_standalone_server_is_refused_with_the_recipe
  - tests/cli_infobase_copy.rs::a_failed_load_into_the_cluster_names_the_image_and_hides_the_passwords
  - tests/cli_infobase_copy.rs::a_failed_load_into_a_file_base_hides_the_passwords
---

# `infobase create --from` копирует базу

`infobase create --from <база>` создаёт базу этой рабочей копии как копию другой базы,
объявленной по имени в местном слое, с её данными и конфигурацией: снимает с источника образ
DT Конфигуратором (`/DumpIB`) в `workPath/copies/<база>.dt` и поднимает из него новую базу
Конфигуратором — файловую `/RestoreIB`, базу в кластере `CREATEINFOBASE`, затем `/RestoreIB`.
Новую базу на автономном сервере команда не создаёт: отказ рода подбора называет рецепт
`ibcmd server config init` и `ibcmd infobase create`, и снимок не начинается. Источник, не
объявленный в местном слое, файловый источник без базы на месте и база, которую команда
создаёт, — отказ до платформы.

Источник читается как база целиком: его замок берёт граница команды вслед за замками своей
базы и держит до конца команды; источник, который держит другая команда, — отказ
`infobase_busy` до снимка, а команды копии-владельца, пока идёт копия, получают тот же отказ.
В метку источника копия не пишется (`INV.USE-CASES.READING-A-BASE-MAKES-NO-OWNER`), а новая
база записывается в метку за этой копией. Ответ называет источник и путь снимка
([форма](../wire/infobase-create-data.md)).

Превью и ответ копии в кластере предупреждают, как у `infobase create`
(`INV.CLI.INFOBASE-CREATE-FOLLOWS-THE-TARGET-KIND`), что `CrSQLDB=Y` молча берёт
существующую базу данных с тем же именем, и сверх того — что её данные заменит образ
источника. Отказа это не даёт: решение владельца от 07.10.2026 — предупредить, дальше
ответственность разработчика.

Неудачная загрузка образа в созданную базу кластера оставляет её пустой, и отказ называет
это и `infobase restore --input <образ> --replace`. Пароли новой базы и источника в
отказах загрузки скрыты.

Пользователи новой базы — пользователи источника. Новую файловую базу команда спрашивает о
поколении конфигурации; нет ответа — предупреждение, которое называет пользователей
источника и секцию базы, куда объявить их учётные данные.

Файловая база из образа через `ibcmd` ждёт его исполнителя
(`INV.CLI.A-FILE-COPY-IS-CREATED-BY-IBCMD`), база в кластере по шаблону .dt — замера
(`INV.CLI.A-CLUSTER-COPY-IS-CREATED-FROM-THE-DT-TEMPLATE`).

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies),
[`cli.html#map`](../../../docs/site/cli.html#map), замер «Загрузка информационной базы из
DT» и [#181](../../../references/1c/confirmed-runtime-measurements.md).
