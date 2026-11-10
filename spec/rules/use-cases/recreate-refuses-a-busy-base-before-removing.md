---
id: INV.USE-CASES.RECREATE-REFUSES-A-BUSY-BASE-BEFORE-REMOVING
check:
  - tests/cli_infobase_recreate.rs::a_recreate_of_a_held_base_is_refused_with_its_sessions_and_removes_nothing
  - tests/cli_infobase_recreate.rs::a_recreate_of_a_base_held_by_another_command_is_refused_as_busy
  - src/use_cases/managed_stand/cluster.rs::a_managed_recreate_with_sessions_in_the_cluster_is_refused_before_any_change
  - src/use_cases/managed_stand/cluster.rs::a_connected_database_refuses_the_recreate_and_keeps_the_registration
  - src/platform/file_infobase.rs::a_base_without_its_data_file_is_absent_and_a_closed_one_is_free
---

# Пересоздание занятой базы отказывает до удаления

`infobase create --recreate` идёт в таком порядке, и каждый отказ приходит раньше, чем тронута
база:

1. Замок `workPath`, затем замок базы. Базу держит другая команда раннера или её живой агент,
   подключённый к базе, — отказ `infobase_busy`.
2. Проверки: вид цели (`INV.USE-CASES.A-MANAGED-RECREATE-DROPS-ONLY-ITS-OWN-DATABASE`),
   остаток прошлого пересоздания (`INV.USE-CASES.RECREATE-ROLLS-BACK-A-FAILED-CREATION`),
   работающий свой автономный сервер (`INV.USE-CASES.A-BASE-IS-NOT-CREATED-UNDER-ITS-RUNNING-SERVER`)
   и сеансы базы.
3. Снятие простаивающих живых агентов этой базы.
4. Точка невозврата: память о базе стирается, база откладывается.

Сеансы базы — отказ с кодом `infobase_sessions` рода `workspace`, выход 3, с полем
`error.sessions`: `base`, `list`, `from_this_machine`, `terminable: false` и `lock`. Раннер
сеансы для пересоздания не завершает и `--sessions force` не предлагает. Сеансы узнаются так:

- файловая база — её файл данных держит другой процесс (на Windows `1Cv8.1CD` не открыть без
  разделения доступа, на POSIX на нём чужая блокировка `fcntl`); перечня у файловой базы нет,
  `list: null`, а `lock` — по файлу `1Cv8.cdn`;
- кластер раннера — `rac session list` базы, перечень с полями сеанса; отказ `rac infobase
  drop`, за которым сеансы есть, — тот же отказ;
- своя БД — отказ PostgreSQL переименовать её из-за подключений (SQLSTATE `55006`), `list:
  null`; снятая регистрация возвращается.

Источник: замер [#597](../../../references/1c/confirmed-runtime-measurements.md), разделы M1,
M3 и M4.
