---
id: INV.USE-CASES.MANAGED-DBMS-OWNERSHIP
check:
  - src/use_cases/managed_stand/standalone.rs::a_server_step_postgres_stops_with_its_server
  - src/use_cases/managed_stand/standalone.rs::an_explicit_postgres_outlives_its_server
  - src/use_cases/managed_stand/standalone.rs::a_later_dbms_start_makes_a_step_postgres_explicit
  - src/use_cases/managed_stand/standalone.rs::restart_leaves_the_postgres_alone
  - src/use_cases/managed_stand/standalone.rs::a_failed_server_stops_only_what_this_command_raised
  - src/use_cases/managed_stand/standalone.rs::a_crashed_server_still_takes_its_step_postgres_down
  - src/use_cases/managed_stand/standalone.rs::a_fallen_postgres_under_a_live_server_is_named_and_comes_back_explicit_on_its_port
  - src/use_cases/managed_stand/standalone.rs::after_a_reboot_the_server_starts_again_with_its_postgres_as_a_step
  - src/use_cases/managed_stand/standalone.rs::after_a_reboot_an_explicit_postgres_raised_by_the_server_becomes_its_step
  - src/use_cases/managed_stand/standalone.rs::a_cancelled_start_over_postgres_stops_the_server_and_the_postgres_it_raised
  - src/use_cases/managed_stand/standalone.rs::a_server_that_does_not_stop_keeps_its_record_and_its_postgres_and_a_repeat_goes_on
  - src/use_cases/managed_stand/cluster.rs::stop_takes_ras_then_the_agent_then_the_step_postgres
  - src/use_cases/managed_stand/cluster.rs::an_explicit_postgres_outlives_its_cluster
  - src/use_cases/managed_stand/cluster.rs::a_later_dbms_start_makes_a_step_postgres_explicit
  - src/use_cases/managed_stand/cluster.rs::restart_leaves_the_postgres_alone
  - src/use_cases/managed_stand/cluster.rs::a_failed_cluster_stops_only_what_this_command_raised
  - src/use_cases/managed_stand/cluster.rs::stop_leaves_the_step_postgres_to_the_running_standalone
  - src/use_cases/managed_stand/cluster.rs::a_crashed_cluster_still_takes_its_step_postgres_down
  - src/use_cases/managed_stand/cluster.rs::a_fallen_postgres_under_a_live_cluster_is_named_and_comes_back_explicit_on_its_port
  - src/use_cases/managed_stand/cluster.rs::a_cancelled_cluster_start_stops_the_postgres_it_raised
  - src/use_cases/managed_stand/cluster.rs::after_a_reboot_the_cluster_starts_again_with_its_postgres_as_a_step
  - src/use_cases/managed_stand/cluster.rs::after_a_reboot_the_command_that_raises_the_postgres_decides_whether_it_is_explicit
  - src/use_cases/managed_stand/cluster.rs::a_remnant_that_does_not_stop_keeps_the_agent_record
  - src/use_cases/managed_stand/mod.rs::stop_refuses_under_a_running_own_server_and_names_it
  - src/use_cases/managed_stand/mod.rs::a_stale_record_is_replaced_and_named_and_the_data_stays
  - src/use_cases/managed_stand/mod.rs::stop_that_failed_keeps_the_record_and_a_repeat_goes_on
  - src/use_cases/managed_stand/mod.rs::a_foreign_process_on_the_pinned_port_is_named_and_left_alone
  - src/use_cases/managed_stand/mod.rs::status_and_stop_work_from_the_record_when_the_tool_key_is_gone
---

# Кто поднял СУБД, тот её и гасит

Запись о СУБД помнит, поднята ли она явно, командой `dbms start`, или шагом `cluster start` либо `standalone start`.

| Ситуация | Что происходит |
| --- | --- |
| СУБД поднята шагом сервера | Парный `stop` гасит сервер и СУБД |
| После этого вызван `dbms start` | СУБД становится явной, `stop` сервера её не трогает |
| `dbms stop` или `dbms restart` при работающем своём сервере | Отказ с названием сервера |
| `cluster restart`, `standalone restart` | СУБД не трогают |
| `cluster stop`, пока над той же СУБД работает свой автономный сервер этой базы | Гасит кластер, а СУБД оставляет серверу |
| Отмена или сбой после подъёма СУБД | Гасится только то, что подняла эта команда |
| Сервер упал, неявная СУБД жива | Парный `stop` гасит СУБД и снимает запись о запущенном сервере |
| СУБД упала, свой сервер жив | `dbms status` называет падение, `dbms start` поднимает её на прежнем порту, и она становится явной |
| `stop` сорвался на части процессов | Записи живых процессов остаются, ответ их называет, повтор `stop` продолжает |
| Запись не совпала с процессом, например после перезагрузки | Служба считается остановленной, следующий `start` поднимает её, и явность определяет эта команда |
| На порту записи работает чужой процесс | `stop` успешен и называет его, `start` отказывает и называет его |
| `tools.postgres.path` больше не объявлен | `status` и `stop` работают по пути из записи |

Снятие и замена записи о запущенной службе не отменяют закреплённых портов: `INV.PLATFORM.MANAGED-PORTS-ARE-PINNED`.
