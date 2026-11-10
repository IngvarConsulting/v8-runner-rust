---
id: INV.USE-CASES.RECREATE-REFUSES-A-BUSY-BASE-BEFORE-REMOVING
check:
  - tests/cli_infobase_recreate.rs::a_recreate_of_a_held_base_is_refused_with_its_sessions_and_removes_nothing
  - tests/cli_infobase_recreate.rs::a_recreate_of_a_base_held_by_another_command_is_refused_as_busy
  - src/use_cases/managed_stand/cluster.rs::a_managed_recreate_with_sessions_in_the_cluster_is_refused_before_any_change
  - src/use_cases/managed_stand/cluster.rs::a_connected_database_refuses_the_recreate_and_keeps_the_registration
---

# Пересоздание занятой базы отказывает до удаления

`infobase create --recreate` идёт в таком порядке, и каждый отказ приходит раньше, чем тронута
база:

1. Замок `workPath`, затем замок базы. Базу держит другая команда раннера или её живой агент,
   подключённый к базе, — отказ `infobase_busy`.
2. Проверки: вид цели (`INV.USE-CASES.A-MANAGED-RECREATE-DROPS-ONLY-ITS-OWN-DATABASE`),
   остаток прошлого пересоздания (`INV.USE-CASES.RECREATE-ROLLS-BACK-A-FAILED-CREATION`),
   работающий свой автономный сервер (`INV.USE-CASES.A-BASE-IS-NOT-CREATED-UNDER-ITS-RUNNING-SERVER`),
   превью создания и сеансы базы.
3. Снятие простаивающих живых агентов этой базы.
4. Точка невозврата: база откладывается, затем стирается память о ней.

Сеансы базы — отказ `infobase_sessions` той же формы, что у применения
(`INV.CLI.APPLY-SESSIONS-GOVERNS-FOREIGN-SESSIONS`), с `terminable: false`: для пересоздания
раннер сеансы не завершает и `--sessions force` не предлагает. Сеансы узнаются так:

- файловая база — её файл данных держит другой процесс; проба повторяется прямо перед переносом
  файлов;
- кластер раннера — `rac session list` базы, перечень с кодом приложения из `app-id`; отказ
  `rac infobase drop`, за которым сеансы есть, — тот же отказ;
- своя БД — отказ PostgreSQL переименовать её из-за подключений (SQLSTATE `55006`), `list:
  null`; снятая регистрация возвращается.

Источник: замер [#597](../../../references/1c/confirmed-runtime-measurements.md), разделы M1,
M3 и M4.
