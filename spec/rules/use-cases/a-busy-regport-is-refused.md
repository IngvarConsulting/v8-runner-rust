---
id: INV.USE-CASES.A-BUSY-REGPORT-IS-REFUSED
check:
  - src/use_cases/managed_stand/cluster.rs::a_busy_regport_is_refused_with_its_process_and_no_new_port_is_taken
  - src/use_cases/managed_stand/cluster.rs::a_foreign_agent_over_this_registry_is_refused_without_a_new_registry
  - src/use_cases/managed_stand/cluster.rs::new_regport_moves_the_base_to_a_new_registry_over_its_database
  - src/use_cases/managed_stand/cluster.rs::a_failed_move_keeps_the_old_registry_and_its_ports
  - src/use_cases/managed_stand/cluster.rs::new_regport_on_free_ports_is_refused
---

# Занятый `regport` не заменяется молча

`cluster start` не берёт другой `regport` сам: занятый чужим процессом закреплённый порт — `regport`, порт агента или
порт диапазона рабочих процессов — отказ с его названием и ключом `--new-regport`. Если на нём работает чужой `ragent`
над реестром этой копии, отказ ключа не предлагает: такой реестр живой. `--new-regport` при свободных портах — отказ. Этот ключ заводит новый реестр на новых портах и регистрирует объявленную базу без `--create-database`
над прежней БД: UUID базы меняется, данные и поколение конфигурации остаются. Была ли база, раннер знает по признаку
создания в прежнем реестре, а у своего PostgreSQL — и по самой БД. Если базы ещё нет, остаётся только
новый реестр. Прежний реестр удаляется после регистрации и успешной пробы базы, поэтому отказ на этом шаге ничего не теряет.
