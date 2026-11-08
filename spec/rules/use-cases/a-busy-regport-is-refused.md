---
id: INV.USE-CASES.A-BUSY-REGPORT-IS-REFUSED
check:
  - src/use_cases/managed_stand/cluster.rs::a_busy_regport_is_refused_with_its_process_and_no_new_port_is_taken
  - src/use_cases/managed_stand/cluster.rs::a_foreign_agent_over_this_registry_is_refused_without_a_new_registry
  - src/use_cases/managed_stand/cluster.rs::new_regport_moves_the_base_to_a_new_registry_over_its_database
  - src/use_cases/managed_stand/cluster.rs::a_failed_move_keeps_the_old_registry_and_its_ports
  - src/use_cases/managed_stand/cluster.rs::new_regport_on_free_ports_is_refused
  - src/use_cases/managed_stand/cluster.rs::a_busy_regport_is_refused_before_the_postgres_starts
  - src/use_cases/managed_stand/cluster.rs::a_taken_port_without_the_process_list_is_refused_and_nothing_is_removed
  - src/use_cases/managed_stand/cluster.rs::start_rolls_back_an_interrupted_move
  - src/use_cases/managed_stand/cluster.rs::a_move_whose_new_agent_does_not_stop_is_left_to_the_next_start
  - src/use_cases/managed_stand/cluster.rs::a_foreign_agent_over_this_registry_on_other_ports_is_refused
---

# Занятый `regport` не заменяется молча

`cluster start` не берёт другой `regport` сам: занятый чужим процессом закреплённый порт — `regport`, порт агента или
порт диапазона рабочих процессов — отказ с его названием и ключом `--new-regport`. Если на нём работает чужой `ragent`
над реестром этой копии — на этих портах или на других, — отказ ключа не предлагает: такой реестр живой. Если перечня процессов нет, кто держит порт, не
сказать — отказ без удаления чего-либо. Эти отказы приходят до запуска PostgreSQL. `--new-regport` при свободных
портах — отказ. Этот ключ заводит новый реестр на новых портах и регистрирует объявленную базу без `--create-database`
над прежней БД: UUID базы меняется, данные и поколение конфигурации остаются. Была ли база, раннер знает по признаку
создания в прежнем реестре, а у своего PostgreSQL — и по самой БД. Если базы ещё нет, остаётся только
новый реестр. Прежний реестр удаляется после регистрации и успешной пробы базы, поэтому отказ на этом шаге ничего не теряет.
Закреплённое помнит прежний реестр, пока перенос не закончен: прерванный перенос следующий `cluster start` откатывает к
прежним портам, а рабочие команды и `infobase create` такой кластер не видят. Если агент нового реестра при неудаче
не снялся, перенос так и остаётся записанным до следующего `cluster start`.
